//! Explicit asynchronous, LIFO teardown with retained outcomes.
//!
//! This is NOT asynchronous RAII. The caller must await `close`; dropping its
//! future aborts its directly owned cleanup task but cannot await completion.
//! Hooks must not detach work or block runtime threads.

use crate::{BoxError, ConfigurationError, RegistrationError, scoped_dispatch, validation};
use std::{future::Future, pin::Pin, time::Duration};
use tokio::{
    task::{JoinError, JoinSet},
    time::Instant,
};
use tracing::Instrument;

/// Work budget plus a final bounded allowance for observing abortion.
#[derive(Clone, Copy, Debug)]
pub struct CleanupBudget {
    total: Duration,
    per_hook: Duration,
    abort_reap: Duration,
}

impl CleanupBudget {
    /// Total hook work, maximum work per hook, and abort-observation allowance.
    /// All must be positive. Hooks share the total budget; it does not reset.
    pub fn new(
        total: Duration,
        per_hook: Duration,
        abort_reap: Duration,
    ) -> Result<Self, ConfigurationError> {
        validation::positive(total, "cleanup total")?;
        validation::positive(per_hook, "cleanup per hook")?;
        validation::positive(abort_reap, "cleanup abort reap")?;
        let combined = total
            .checked_add(abort_reap)
            .ok_or(ConfigurationError::TooLarge("cleanup combined budget"))?;
        validation::representable(combined, "cleanup combined budget")?;
        Ok(Self {
            total,
            per_hook,
            abort_reap,
        })
    }

    /// Cooperative-runtime upper bound for this cleanup phase.
    pub fn total_allowance(&self) -> Duration {
        self.total + self.abort_reap
    }
}

/// A single finalizer's observed outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CleanupOutcome {
    /// Completed successfully.
    Succeeded,
    /// Returned an error, retained in its record.
    Failed,
    /// The finalizer or its factory panicked.
    Panicked,
    /// Cancelled by the runtime without a library timeout winning first.
    Cancelled,
    /// Timed out; the directly owned task was subsequently joined.
    TimedOut,
    /// Abort was requested but task completion was not observed in time.
    Unjoined,
}

/// Why a registered finalizer was not started.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    /// The shared cleanup work budget was exhausted.
    BudgetExhausted,
    /// An earlier finalizer may still be using a dependency.
    PreviousCleanupUnjoined,
    /// Process task termination was not safe enough for dependent teardown.
    UnsafeTaskExit,
}

/// A retained cleanup result. Error contents are never automatically logged.
#[derive(Debug)]
pub struct CleanupRecord {
    /// Developer-controlled name.
    pub name: &'static str,
    /// Observed outcome.
    pub outcome: CleanupOutcome,
    /// Original error or JoinError when available.
    pub error: Option<BoxError>,
}

impl CleanupRecord {
    fn log_observation(&self) {
        if self.outcome == CleanupOutcome::Succeeded {
            tracing::info!(target: "batter", cleanup = self.name, outcome = ?self.outcome, "cleanup observed");
        } else {
            tracing::warn!(target: "batter", cleanup = self.name, outcome = ?self.outcome, "cleanup observed");
        }
    }
}

/// A cleanup hook intentionally not invoked.
#[derive(Debug)]
pub struct SkippedCleanup {
    /// Developer-controlled name.
    pub name: &'static str,
    /// Explicit reason for omission.
    pub reason: SkipReason,
}

/// All teardown outcomes; a first error does not conceal later errors.
/// Inspect the report even after awaiting cleanup: completion can contain failures.
///
/// ```compile_fail
/// #![deny(unused_must_use)]
/// use batter::cleanup::{CleanupBudget, CleanupStack};
/// async fn ignored(stack: CleanupStack, budget: CleanupBudget) {
///     stack.close(budget).await;
/// }
/// ```
#[derive(Debug, Default)]
#[must_use = "inspect the report for failures and incomplete cleanup"]
pub struct CleanupReport {
    /// Attempted hooks in LIFO order.
    pub records: Vec<CleanupRecord>,
    /// Hooks never invoked, also in LIFO order.
    pub skipped: Vec<SkippedCleanup>,
}

impl CleanupReport {
    /// True only if every registered hook completed successfully.
    pub fn is_success(&self) -> bool {
        self.skipped.is_empty()
            && self
                .records
                .iter()
                .all(|r| r.outcome == CleanupOutcome::Succeeded)
    }
}

impl std::fmt::Display for CleanupReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let failed = self
            .records
            .iter()
            .filter(|r| r.outcome != CleanupOutcome::Succeeded)
            .count();
        write!(
            f,
            "cleanup: {failed} unsuccessful, {} skipped",
            self.skipped.len()
        )
    }
}
impl std::error::Error for CleanupReport {}

type CleanupFuture = Pin<Box<dyn Future<Output = Result<(), BoxError>> + Send + 'static>>;
struct Hook {
    name: &'static str,
    action: Box<dyn FnOnce() -> CleanupFuture + Send + 'static>,
}

/// Finalizers registered immediately after successful acquisition.
///
/// Register dependencies first and their dependents later: close reverses
/// registration order. No hook is run by Drop. Values captured by hooks must
/// be owned and Send; borrowed resources need application-owned teardown.
#[derive(Default)]
pub struct CleanupStack {
    hooks: Vec<Hook>,
}

/// A validated name and exclusive registration slot, reserved before acquisition.
///
/// Dropping an unused slot registers nothing. After acquisition succeeds, call
/// [`Self::register`] without an intervening await. It cannot reject the name.
/// This does not own an acquisition future or finalize an unregistered resource.
///
/// ```
/// use batter::cleanup::CleanupStack;
/// # async fn example() -> Result<(), batter::RegistrationError> {
/// let mut cleanup = CleanupStack::new();
/// let slot = cleanup.reserve("resource")?;
/// let resource = String::from("acquired");
/// slot.register(move || async move { drop(resource); Ok(()) });
/// # let budget = batter::cleanup::CleanupBudget::new(std::time::Duration::from_secs(1), std::time::Duration::from_secs(1), std::time::Duration::from_secs(1)).unwrap();
/// # assert!(cleanup.close(budget).await.is_success());
/// # Ok(()) }
/// ```
#[must_use = "register acquired cleanup, or drop the unused reservation"]
pub struct CleanupSlot<'a> {
    stack: &'a mut CleanupStack,
    name: &'static str,
}

impl CleanupSlot<'_> {
    /// Register an owned finalizer under the already validated name.
    pub fn register<F, Fut>(self, action: F)
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = Result<(), BoxError>> + Send + 'static,
    {
        self.stack.hooks.push(Hook {
            name: self.name,
            action: Box::new(move || Box::pin(action()) as CleanupFuture),
        });
    }
}

impl CleanupStack {
    /// An empty stack, with no background tasks.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a factory. It is only invoked inside a supervised cleanup task.
    pub fn push<F, Fut>(&mut self, name: &'static str, action: F) -> Result<(), RegistrationError>
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = Result<(), BoxError>> + Send + 'static,
    {
        self.reserve(name)?.register(action);
        Ok(())
    }

    /// Validate before acquisition and hold exclusive registration authority.
    ///
    /// A rejected reservation consumes no finalizer or resource. Existing
    /// [`Self::push`] retains its original consume-on-error behavior; use this
    /// method when acquisition must not precede a possible registration failure.
    pub fn reserve(&mut self, name: &'static str) -> Result<CleanupSlot<'_>, RegistrationError> {
        validation::name(name)?;
        if self.hooks.iter().any(|hook| hook.name == name) {
            return Err(RegistrationError::Duplicate(name));
        }
        Ok(CleanupSlot { stack: self, name })
    }

    /// Number of pending hooks.
    pub fn len(&self) -> usize {
        self.hooks.len()
    }

    /// Whether there is any registered teardown.
    pub fn is_empty(&self) -> bool {
        self.hooks.is_empty()
    }

    /// Record that dependent teardown cannot safely proceed.
    pub fn skip(mut self, reason: SkipReason) -> CleanupReport {
        let mut report = CleanupReport::default();
        self.skip_remaining(&mut report, reason);
        report
    }

    fn skip_remaining(&mut self, report: &mut CleanupReport, reason: SkipReason) {
        while let Some(hook) = self.hooks.pop() {
            tracing::warn!(target: "batter", cleanup = hook.name, ?reason, "cleanup skipped");
            report.skipped.push(SkippedCleanup {
                name: hook.name,
                reason,
            });
        }
    }

    /// Await LIFO teardown. Continue after returned errors and joined panics.
    ///
    /// On timeout, abort and attempt to join the directly owned hook. If that
    /// join cannot be observed, skip dependent hooks rather than claiming
    /// teardown is safe. Abort observation shares a final absolute bound of
    /// `start + total + abort_reap`; it cannot extend indefinitely per hook.
    /// Every skipped hook is reported and logged once, in LIFO order. Polling
    /// and destruction retain the tracing subscriber captured on first poll.
    pub async fn close(self, budget: CleanupBudget) -> CleanupReport {
        scoped_dispatch::scope(self.close_inner(budget)).await
    }

    async fn close_inner(mut self, budget: CleanupBudget) -> CleanupReport {
        let started = Instant::now();
        let work_deadline = started + budget.total;
        let final_deadline = started + budget.total_allowance();
        let mut report = CleanupReport::default();
        loop {
            // Keep skipped hooks on the stack so reporting and capture drops
            // follow the same reverse registration order for every skip reason.
            if Instant::now() >= work_deadline {
                self.skip_remaining(&mut report, SkipReason::BudgetExhausted);
                break;
            }
            let Some(hook) = self.hooks.pop() else {
                break;
            };
            let name = hook.name;
            let deadline = work_deadline.min(Instant::now() + budget.per_hook);
            let mut running = JoinSet::new();
            let span = tracing::info_span!(target: "batter", "batter.cleanup", cleanup = name);
            // The factory is called INSIDE the task so its panic is observable.
            running.spawn(scoped_dispatch::scope(
                async move { (hook.action)().await }.instrument(span),
            ));
            let mut observation = PendingCleanupObservation {
                name,
                observed: false,
            };
            let completed = tokio::select! {
                biased;
                result = running.join_next() => result,
                _ = tokio::time::sleep_until(deadline) => None,
            };
            let record = match completed {
                Some(result) => cleanup_record(name, result),
                None => abort_cleanup(&mut running, name, final_deadline, budget.abort_reap).await,
            };
            let unjoined = record.outcome == CleanupOutcome::Unjoined;
            observation.observed = true;
            record.log_observation();
            report.records.push(record);
            if unjoined {
                self.skip_remaining(&mut report, SkipReason::PreviousCleanupUnjoined);
                break;
            }
        }
        report
    }
}

fn cleanup_record(
    name: &'static str,
    result: Result<Result<(), BoxError>, JoinError>,
) -> CleanupRecord {
    match result {
        Ok(Ok(())) => CleanupRecord {
            name,
            outcome: CleanupOutcome::Succeeded,
            error: None,
        },
        Ok(Err(error)) => CleanupRecord {
            name,
            outcome: CleanupOutcome::Failed,
            error: Some(error),
        },
        Err(error) => CleanupRecord {
            name,
            outcome: if error.is_panic() {
                CleanupOutcome::Panicked
            } else {
                CleanupOutcome::Cancelled
            },
            error: Some(Box::new(error)),
        },
    }
}

async fn abort_cleanup(
    running: &mut JoinSet<Result<(), BoxError>>,
    name: &'static str,
    final_deadline: Instant,
    abort_reap: Duration,
) -> CleanupRecord {
    running.abort_all();
    let reap_deadline = final_deadline.min(Instant::now() + abort_reap);
    let joined = tokio::select! {
        biased;
        result = running.join_next() => result,
        _ = tokio::time::sleep_until(reap_deadline) => None,
    };
    match joined {
        Some(result) => {
            let mut record = cleanup_record(name, result);
            record.outcome = CleanupOutcome::TimedOut;
            record
        }
        None => CleanupRecord {
            name,
            outcome: CleanupOutcome::Unjoined,
            error: None,
        },
    }
}

struct PendingCleanupObservation {
    name: &'static str,
    observed: bool,
}

impl Drop for PendingCleanupObservation {
    fn drop(&mut self) {
        if !self.observed {
            tracing::warn!(target: "batter", cleanup = self.name, "cleanup driver dropped before hook result was observed");
        }
    }
}

impl Drop for CleanupStack {
    fn drop(&mut self) {
        if !self.hooks.is_empty() {
            tracing::warn!(target: "batter", pending_hooks = self.hooks.len(), "cleanup stack dropped without close; asynchronous hooks were NOT run");
        }
    }
}
