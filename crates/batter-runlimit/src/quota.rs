//! Admission owns entry into a work factory, preserving native results.

use batter_core::operation::{Interruption, OperationContext, OperationError};
use runlimit_core::{
    Allowance, BatchDecision, BatchDecisionView, Check, ConsumptionStatus, Denial, Limiter,
    QuotaDenial, RateLimitPolicy,
};
use std::{future::Future, num::NonZeroUsize, pin::Pin, sync::Arc};

// Native checks can own pool acquisition and transactions (PostgreSQL); erase
// them once so their state and type depth never enter the consumer's future.
type NativeCheck<'a, E> = Pin<Box<dyn Future<Output = Result<BatchDecision, E>> + Send + 'a>>;
// Admitted work often nests further adapters (atomic runners, queries). Erasing
// it once ends the caller's layout and `Send` proof at this boundary; the work
// future's own `Send` proof is checked separately where it is created.
type AdmittedWork<'a, T, E> = Pin<Box<dyn Future<Output = Result<T, E>> + Send + 'a>>;

/// A narrow bridge; native policy, storage, and error types stay upstream.
pub trait ConsumptionError: std::error::Error + Send + Sync + 'static {
    /// Native evidence about consumption; unknown variants must stay uncertain.
    fn consumption(&self) -> ConsumptionStatus;
}

#[cfg(feature = "memory")]
impl ConsumptionError for runlimit_memory::MemoryBatchError {
    fn consumption(&self) -> ConsumptionStatus {
        use runlimit_memory::MemoryBatchError::*;
        match self {
            InvalidBatch(_) | BatchExceedsShardCapacity { .. } | PoisonedShard { .. } => {
                ConsumptionStatus::NotConsumed
            }
        }
    }
}

#[cfg(feature = "postgres")]
impl ConsumptionError for runlimit_postgres::BatchCheckError {
    fn consumption(&self) -> ConsumptionStatus {
        self.consumption()
    }
}

#[cfg(feature = "memory")]
impl ConsumptionError for runlimit_memory::GcraBatchError {
    fn consumption(&self) -> ConsumptionStatus {
        match self {
            Self::Store(error) => error.consumption(),
            Self::ArithmeticOverflow => ConsumptionStatus::NotConsumed,
        }
    }
}

/// The protected path cannot accidentally admit an empty set of quotas.
pub struct Checks<'a, P: RateLimitPolicy> {
    checks: &'a [Check<'a, P>],
}

/// A protected quota boundary requires at least one native check.
#[derive(Debug, thiserror::Error)]
#[error("at least one quota check is required")]
pub struct EmptyChecks;

impl<'a, P: RateLimitPolicy> Checks<'a, P> {
    /// Require at least one native check. Upstream validates all batch semantics.
    /// See the crate example for policy, subject and batch construction.
    pub fn new(checks: &'a [Check<'a, P>]) -> Result<Self, EmptyChecks> {
        if checks.is_empty() {
            return Err(EmptyChecks);
        }
        Ok(Self { checks })
    }
}

/// Internal observations for the retained HTTP quota recorder.
#[derive(Clone, Debug)]
#[cfg_attr(
    not(feature = "axum"),
    expect(
        dead_code,
        reason = "terminal fields are read only by the HTTP recorder"
    )
)]
pub(crate) enum Snapshot {
    /// A native check started without an observed result; consumption is unknown.
    Started,
    /// An observed native atomic decision.
    Decided(BatchDecision),
    /// An observed native failure's consumption certainty.
    Failed(ConsumptionStatus),
}

/// Progress known when quota admission was interrupted.
///
/// In-flight does not prove that local or remote work still runs after the
/// check future was dropped; consumption remains unknown.
///
/// ```
/// use batter_runlimit::{InterruptedCheck, RunResult};
/// fn interrupted_check<T, E, B>(result: &RunResult<T, E, B>) -> Option<InterruptedCheck> {
///     match result {
///         RunResult::Interrupted { check, .. } => Some(*check),
///         _ => None,
///     }
/// }
/// ```
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InterruptedCheck {
    /// No native check was invoked.
    NotStarted,
    /// The native check was invoked without an observed result.
    InFlight,
}

/// A native batch already restricted to consumed allowances.
///
/// Runlimit's `Allowance` type makes denied members unrepresentable. The
/// values remain in caller-supplied check order.
#[derive(Debug)]
pub struct AllowedBatch(Vec<Allowance>);

impl AllowedBatch {
    /// Number of allowances retained from the native batch.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether the native batch contained no allowances.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Inspect each native allowance in caller-supplied order.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = Allowance> + '_ {
        self.0.iter().copied()
    }
}

/// A native batch result that permitted application work.
#[derive(Debug)]
pub enum Admission {
    /// All checks were allowed and consumed in caller-supplied order.
    Allowed {
        /// One native allowance per supplied check.
        allowances: AllowedBatch,
    },
    /// A native quota denial allowed work under an explicit shadow policy.
    ShadowDenied {
        /// Index of the denied check in the original batch.
        index: usize,
        /// Validated number of checks in the evaluated batch.
        batch_size: NonZeroUsize,
        /// Native validated quota-denial details.
        denial: QuotaDenial,
    },
}

/// A quota grant is kept separate from the outcome of the admitted application work.
#[must_use]
#[derive(Debug)]
pub enum RunResult<T, E, B> {
    /// An enforced native decision prevented factory invocation.
    Rejected {
        /// Index of the denied check in the original batch.
        index: usize,
        /// Validated number of checks in the evaluated batch.
        batch_size: NonZeroUsize,
        /// Native denial details, including any retry lower bound.
        denial: Denial,
    },
    /// Native failure prevented work; no automatic retry follows.
    Backend {
        /// Concrete native failure, never automatically formatted or logged.
        error: B,
        /// Independently retained consumption evidence.
        consumption: ConsumptionStatus,
    },
    /// Admission was interrupted; in-flight does not mean local work still runs.
    Interrupted {
        /// Cancellation or total-deadline exhaustion.
        reason: Interruption,
        /// Whether the native check had started before interruption.
        check: InterruptedCheck,
    },
    /// Quota permitted work; cancellation may still prevent factory invocation.
    Admitted {
        /// Narrowed native result that permitted work.
        admission: Admission,
        /// Work result under the same parent budget, preserving concrete errors.
        work: Result<T, OperationError<E>>,
    },
}

/// Shared native limiter whose execution path owns admission before work.
/// Construction is inert. Native resource initialization remains upstream-owned.
pub struct Quota<L> {
    limiter: Arc<L>,
}

impl<L> Clone for Quota<L> {
    fn clone(&self) -> Self {
        Self {
            limiter: self.limiter.clone(),
        }
    }
}

impl<L: Limiter> Quota<L>
where
    L::CheckAllError: ConsumptionError,
{
    /// Share a prepared native limiter. See the crate-level runnable example.
    pub fn new(limiter: L) -> Self {
        Self {
            limiter: Arc::new(limiter),
        }
    }

    /// One native atomic admission; the same parent budget then bounds work.
    ///
    /// Constructing the returned future does not call the native limiter. Once
    /// polled, the native check may take effect before yielding. No automatic
    /// retries, spawned work, refundable grant, or streaming lifetime.
    ///
    /// The native check and the admitted work future are each heap-allocated
    /// once, when they start, so this future stays small and its type shallow
    /// inside handlers and tasks that compose further adapters. The work future
    /// must therefore be `Send`, as handler and spawned-task futures already are.
    ///
    /// Migration: `Quota::run` accepted non-`Send` work futures in 0.0.1.
    /// Generic wrappers now need `Fut: Future<Output = Result<T, E>> + Send`.
    /// Replace thread-local captures held by the future with owned `Send` data
    /// or, for shared `Send + Sync` data, `Arc`. The factory itself need not be
    /// `Send`; it may extract owned data before returning its future.
    ///
    /// A future retaining `Rc` is rejected:
    ///
    /// ```compile_fail
    /// # use batter_core::operation::OperationContext;
    /// # use batter_runlimit::{Checks, ConsumptionError, Quota, native::Limiter};
    /// # async fn example<L: Limiter>(quota: &Quota<L>, context: &OperationContext,
    /// #     checks: Checks<'_, L::Policy>) where L::CheckAllError: ConsumptionError {
    /// let shared = std::rc::Rc::new(String::from("record"));
    /// let _ = quota.run(context, checks, move |_| async move {
    ///     tokio::task::yield_now().await;
    ///     Ok::<_, std::convert::Infallible>(shared.len())
    /// }).await;
    /// # }
    /// ```
    ///
    /// Use `Arc` when shared ownership is needed:
    ///
    /// ```no_run
    /// # use batter_core::operation::OperationContext;
    /// # use batter_runlimit::{Checks, ConsumptionError, Quota, native::Limiter};
    /// # async fn example<L: Limiter>(quota: &Quota<L>, context: &OperationContext,
    /// #     checks: Checks<'_, L::Policy>) where L::CheckAllError: ConsumptionError {
    /// let shared = std::sync::Arc::new(String::from("record"));
    /// let _ = quota.run(context, checks, move |_| async move {
    ///     tokio::task::yield_now().await;
    ///     Ok::<_, std::convert::Infallible>(shared.len())
    /// }).await;
    /// # }
    /// ```
    pub async fn run<T, E, F, Fut>(
        &self,
        context: &OperationContext,
        checks: Checks<'_, L::Policy>,
        work: F,
    ) -> RunResult<T, E, L::CheckAllError>
    where
        F: FnOnce(OperationContext) -> Fut,
        Fut: Future<Output = Result<T, E>> + Send,
    {
        batter_core::telemetry::with_current_dispatch(self.run_recorded(
            context,
            checks,
            |_| {},
            |scope| -> AdmittedWork<'_, T, E> { Box::pin(work(scope)) },
        ))
        .await
    }

    /// Internal HTTP integration seam; callers do not coordinate an external recorder.
    pub(crate) async fn run_recorded<T, E, F, Fut, Record>(
        &self,
        context: &OperationContext,
        checks: Checks<'_, L::Policy>,
        mut record: Record,
        work: F,
    ) -> RunResult<T, E, L::CheckAllError>
    where
        F: FnOnce(OperationContext) -> Fut,
        Fut: Future<Output = Result<T, E>>,
        Record: FnMut(&Snapshot),
    {
        let mut interrupted_check = InterruptedCheck::NotStarted;
        let result = context
            .run("quota.check", |_| async {
                interrupted_check = InterruptedCheck::InFlight;
                record(&Snapshot::Started);
                let check: NativeCheck<'_, L::CheckAllError> =
                    Box::pin(self.limiter.check_all(checks.checks));
                let result = check.await;
                match &result {
                    Ok(decision) => record(&Snapshot::Decided(decision.clone())),
                    Err(error) => record(&Snapshot::Failed(error.consumption())),
                }
                result
            })
            .await;
        match result {
            Ok(decision) => {
                let admission = match decision.view() {
                    BatchDecisionView::Denied {
                        index,
                        batch_size,
                        denial,
                    } => {
                        return RunResult::Rejected {
                            index,
                            batch_size,
                            denial,
                        };
                    }
                    BatchDecisionView::Allowed { allowances } => Admission::Allowed {
                        allowances: AllowedBatch(allowances.to_vec()),
                    },
                    BatchDecisionView::ShadowDenied {
                        index,
                        batch_size,
                        denial,
                    } => Admission::ShadowDenied {
                        index,
                        batch_size,
                        denial,
                    },
                };
                RunResult::Admitted {
                    admission,
                    // A new child of the same parent: no reset of the total deadline.
                    // Preflight prevents factory invocation if admission cancelled the parent.
                    work: context.run("quota.work", work).await,
                }
            }
            Err(OperationError::Failed(error)) => RunResult::Backend {
                consumption: error.consumption(),
                error,
            },
            Err(OperationError::Interrupted(reason)) => RunResult::Interrupted {
                reason,
                check: interrupted_check,
            },
        }
    }
}
