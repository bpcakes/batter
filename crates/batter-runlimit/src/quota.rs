//! Admission owns entry into a work factory, preserving native results.

use batter::operation::{Interruption, OperationContext, OperationError};
use runlimit_core::{
    BatchDecision, BatchDecisionView, Check, ConsumptionStatus, Decision, DecisionView, Denial,
    Limiter, QuotaDenial, RateLimitPolicy,
};
use std::{future::Future, sync::Arc, time::Duration};

/// A narrow bridge; native policy, storage, and error types stay upstream.
pub trait ConsumptionError: std::error::Error + Send + Sync + 'static {
    /// Native evidence about consumption; unknown variants must stay uncertain.
    fn consumption(&self) -> ConsumptionStatus;
}

#[cfg(feature = "memory")]
impl ConsumptionError for runlimit_memory::MemoryStoreError {
    fn consumption(&self) -> ConsumptionStatus {
        use runlimit_memory::MemoryStoreError::*;
        match self {
            InvalidBatch(_) | BatchExceedsShardCapacity { .. } | PoisonedShard { .. } => {
                ConsumptionStatus::NotConsumed
            }
        }
    }
}

#[cfg(feature = "postgres")]
impl ConsumptionError for runlimit_postgres::CheckError {
    fn consumption(&self) -> ConsumptionStatus {
        use runlimit_postgres::CheckError::*;
        match self {
            CommittedResponseInvariant => ConsumptionStatus::Consumed,
            InvalidBatch(_)
            | DefinitelyNotConsumed(_)
            | TimedOutBeforeCommit { .. }
            | StorageInvariant(_)
            | ResponseInvariant => ConsumptionStatus::NotConsumed,
            // Forward-compatible uncertainty, never a string-based classification.
            _ => ConsumptionStatus::PossiblyConsumed,
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

/// Native metadata for one consumed allowed check.
///
/// Only a validated native allowed batch can create this value. All fields
/// retain their native meaning without requiring a caller to handle denied
/// decision variants that cannot occur in an allowed batch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AllowedDecision {
    capacity: u64,
    available: u64,
    replenishes_after: Duration,
}

impl AllowedDecision {
    /// Maximum immediately available policy allowance.
    pub fn capacity(self) -> u64 {
        self.capacity
    }

    /// Allowance available after this check consumed quota.
    pub fn available(self) -> u64 {
        self.available
    }

    /// Native time until the policy's full capacity is next available.
    pub fn replenishes_after(self) -> Duration {
        self.replenishes_after
    }
}

/// A native batch already validated to contain only allowed decisions.
///
/// This retains Runlimit's decision vector without copying it. Iteration
/// projects only allowed metadata, so consumers need no impossible denial arm.
#[derive(Debug)]
pub struct AllowedBatch(Vec<Decision>);

impl AllowedBatch {
    /// Number of allowed check decisions retained from the native batch.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether the native batch contained no decisions.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Inspect each allowed decision in caller-supplied order.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = AllowedDecision> + '_ {
        self.0.iter().map(|item| match item.view() {
            DecisionView::Allowed {
                capacity,
                available,
                replenishes_after,
            } => AllowedDecision {
                capacity,
                available,
                replenishes_after,
            },
            DecisionView::Denied { .. } | DecisionView::ShadowDenied { .. } => {
                unreachable!("native allowed batch contains only allowed checks")
            }
        })
    }
}

/// A native batch result that permitted application work.
#[derive(Debug)]
pub enum Admission {
    /// All checks were allowed and consumed in caller-supplied order.
    Allowed {
        /// One native allowed decision per supplied check.
        decisions: AllowedBatch,
    },
    /// A native quota denial allowed work under an explicit shadow policy.
    ShadowDenied {
        /// Index of the denied check in the original batch.
        index: usize,
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
    L::Error: ConsumptionError,
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
    pub async fn run<T, E, F, Fut>(
        &self,
        context: &OperationContext,
        checks: Checks<'_, L::Policy>,
        work: F,
    ) -> RunResult<T, E, L::Error>
    where
        F: FnOnce(OperationContext) -> Fut,
        Fut: Future<Output = Result<T, E>>,
    {
        batter::telemetry::with_current_dispatch(self.run_recorded(context, checks, |_| {}, work))
            .await
    }

    /// Internal HTTP integration seam; callers do not coordinate an external recorder.
    pub(crate) async fn run_recorded<T, E, F, Fut, Record>(
        &self,
        context: &OperationContext,
        checks: Checks<'_, L::Policy>,
        mut record: Record,
        work: F,
    ) -> RunResult<T, E, L::Error>
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
                let result = self.limiter.check_all(checks.checks).await;
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
                    BatchDecisionView::Denied { index, denial } => {
                        return RunResult::Rejected {
                            index,
                            denial: *denial,
                        };
                    }
                    BatchDecisionView::Allowed { .. } => Admission::Allowed {
                        decisions: AllowedBatch(
                            decision
                                .try_into_allowed()
                                .expect("native allowed view must yield allowed decisions"),
                        ),
                    },
                    BatchDecisionView::ShadowDenied { index, denial } => {
                        Admission::ShadowDenied { index, denial }
                    }
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
