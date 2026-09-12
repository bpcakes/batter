use super::ManagedSettlement;
use crate::{BoxError, operation::Interruption, startup::PanicPayload};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::{any::Any, fmt, sync::Arc};
use tokio::{sync::watch, task::JoinError};

/// Original typed native report plus its adapter's termination classification.
/// Classification does not discard native errors, aborted tasks or descendants.
#[derive(Clone)]
pub struct SettlementEvidence {
    report: Arc<dyn Any + Send + Sync>,
    success: bool,
    cleanup: bool,
    pub(super) classification_failure: Option<ManagedFailure>,
}

impl SettlementEvidence {
    pub(super) fn new<R: ManagedSettlement>(report: R) -> Self {
        let report = Arc::new(report);
        let classified = catch_unwind(AssertUnwindSafe(|| {
            (report.is_success(), report.allows_dependency_cleanup())
        }));
        let (success, cleanup, classification_failure) = match classified {
            Ok((success, cleanup)) => (success, cleanup, None),
            Err(payload) => (
                false,
                false,
                Some(ManagedFailure::Panicked(Arc::new(PanicPayload::new(
                    payload,
                )))),
            ),
        };
        Self {
            success,
            cleanup,
            report,
            classification_failure,
        }
    }

    /// Borrow the original adapter report for explicit trusted inspection.
    pub fn downcast_ref<R: ManagedSettlement>(&self) -> Option<&R> {
        self.report.downcast_ref()
    }
}

impl ManagedSettlement for SettlementEvidence {
    fn is_success(&self) -> bool {
        self.success
    }
    fn allows_dependency_cleanup(&self) -> bool {
        self.cleanup
    }
}

impl fmt::Debug for SettlementEvidence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SettlementEvidence")
            .field("success", &self.success)
            .field("allows_dependency_cleanup", &self.cleanup)
            .finish_non_exhaustive()
    }
}

/// Original lifecycle failures. Formatting never includes application contents.
#[derive(Clone)]
pub enum ManagedFailure {
    /// Factory or initialization returned this error.
    Failed(Arc<dyn std::error::Error + Send + Sync>),
    /// Factory, future polling, callback or destruction unwound.
    Panicked(Arc<PanicPayload>),
    /// The independently retained coordinator exited without a complete report.
    Coordinator(Arc<JoinError>),
}

impl fmt::Debug for ManagedFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Failed(_) => "managed application failure",
            Self::Panicked(_) => "managed panic retained",
            Self::Coordinator(_) => "managed coordinator terminated",
        })
    }
}

impl From<BoxError> for ManagedFailure {
    fn from(error: BoxError) -> Self {
        Self::Failed(Arc::from(error))
    }
}

/// Initialization is separate from native settlement and application approval.
#[derive(Clone)]
pub enum ManagedInitialization {
    /// The factory has not run; no native work was constructed through it.
    NotStarted {
        /// Deadline/cancellation that prevented factory invocation, if any.
        interruption: Option<Interruption>,
    },
    /// Native construction rejected configuration without starting native work.
    /// The factory contract forbids returning Err after spawning descendants.
    Rejected(Arc<dyn std::error::Error + Send + Sync>),
    /// Factory execution or initialization is still in progress.
    Pending,
    /// Actual native initialization was acknowledged by the library.
    Initialized,
    /// Process drain won the initialization race.
    Draining,
    /// Native stopping or settlement was observed before initialization and
    /// before process drain. The native report and any failures remain separate.
    ///
    /// ```
    /// use batter::lifecycle::{ManagedInitialization, ManagedOutcome};
    /// fn stopped_before_initialization(outcome: &ManagedOutcome) -> bool {
    ///     matches!(outcome.initialization, ManagedInitialization::Stopped)
    /// }
    /// ```
    Stopped,
    /// Its absolute startup deadline or cancellation prevented initialization.
    Interrupted(Interruption),
    /// Original construction/initialization error or panic.
    Failed(ManagedFailure),
}

impl Default for ManagedInitialization {
    fn default() -> Self {
        Self::NotStarted { interruption: None }
    }
}

impl fmt::Debug for ManagedInitialization {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotStarted { interruption } => f
                .debug_struct("NotStarted")
                .field("interruption", interruption)
                .finish(),
            Self::Rejected(_) => f.write_str("native construction rejected"),
            Self::Pending => f.write_str("native initialization pending"),
            Self::Initialized => f.write_str("native initialized"),
            Self::Draining => f.write_str("native initialization drained"),
            Self::Stopped => f.write_str("native stopped before initialization"),
            Self::Interrupted(reason) => f.debug_tuple("Interrupted").field(reason).finish(),
            Self::Failed(reason) => f.debug_tuple("Failed").field(reason).finish(),
        }
    }
}

/// A snapshot of native initialization, lifecycle failures and settlement.
/// A pending snapshot makes no termination claim, even if its waiter was joined.
#[derive(Clone, Debug, Default)]
#[must_use = "inspect native initialization and settlement before declaring success"]
pub struct ManagedOutcome {
    /// Native initialization result, independent of application approval.
    pub initialization: ManagedInitialization,
    /// Native report once available; it retains the adapter's concrete type.
    pub settlement: Option<SettlementEvidence>,
    /// Additional failures, including independent future-destructor panics.
    pub failures: Vec<ManagedFailure>,
    /// Native stop/exit was observed before process drain.
    pub unexpected_stop: bool,
    /// The independent owner has published its final outcome.
    pub finished: bool,
}

impl ManagedOutcome {
    /// Full managed lifecycle success, not merely waiter completion.
    pub fn is_success(&self) -> bool {
        self.finished
            && self.allows_dependency_cleanup()
            && !self.unexpected_stop
            && self.failures.is_empty()
            && matches!(self.initialization, ManagedInitialization::Initialized)
            && self
                .settlement
                .as_ref()
                .is_some_and(ManagedSettlement::is_success)
    }

    /// Whether dependency cleanup is permitted by the retained evidence.
    /// An initialization error may still permit cleanup after native cooperation;
    /// a panic or uncertain coordinator/native termination never does.
    pub fn allows_dependency_cleanup(&self) -> bool {
        self.finished
            && self.failures.is_empty()
            && !matches!(
                self.initialization,
                ManagedInitialization::Failed(
                    ManagedFailure::Panicked(_) | ManagedFailure::Coordinator(_)
                )
            )
            && (matches!(
                self.initialization,
                ManagedInitialization::NotStarted { .. } | ManagedInitialization::Rejected(_)
            ) || self
                .settlement
                .as_ref()
                .is_some_and(ManagedSettlement::allows_dependency_cleanup))
    }
}

/// Retained observation of the independently driven native component. Waiting
/// never owns or cancels it. A runtime must remain alive to finish settlement.
#[derive(Clone, Debug)]
pub struct ManagedObserver {
    pub(super) completion: watch::Receiver<ManagedOutcome>,
}

impl ManagedObserver {
    /// Snapshot without waiting or changing native ownership.
    pub fn snapshot(&self) -> ManagedOutcome {
        self.completion.borrow().clone()
    }

    /// Wait for final settlement, including after a supervisor's bounded report
    /// marked it pending. This does not retroactively run skipped finalizers.
    /// Runtime death before publication cannot manufacture a complete report.
    pub async fn wait(&self) -> ManagedOutcome {
        let mut completion = self.completion.clone();
        loop {
            let snapshot = completion.borrow_and_update().clone();
            if snapshot.finished {
                return snapshot;
            }
            if completion.changed().await.is_err() {
                return snapshot;
            }
        }
    }

    pub(in super::super) async fn until(&self, deadline: impl std::future::Future<Output = ()>) {
        tokio::select! {
            biased;
            _ = deadline => {},
            _ = self.wait() => {},
        }
    }
}

/// Frozen process-report evidence plus observation of any late native settlement.
#[derive(Clone, Debug)]
pub struct ManagedRecord {
    /// Validated registration name.
    pub name: &'static str,
    /// Evidence at the end of the supervisor's bounded observation interval.
    pub outcome: ManagedOutcome,
    /// Native settlement continues independently if the frozen outcome is pending.
    pub observer: ManagedObserver,
}
