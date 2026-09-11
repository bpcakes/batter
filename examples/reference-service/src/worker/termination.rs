use super::{
    ProbeLeaseRelease, TerminationState, WorkerBuildError, WorkerDriverError, WorkerStartupError,
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU8, Ordering},
};
use tokio::sync::watch;

/// Independently retained outcomes, including failures whose caller was cancelled.
#[derive(Clone, Default)]
pub struct WorkerSettlement {
    /// Preparation failure, also retained when the preparation waiter disappears.
    pub preparation: Option<Arc<WorkerStartupError>>,
    /// Native driver failure, also retained when its registered wrapper disappears.
    pub driver: Option<Arc<WorkerDriverError>>,
    /// Separate server unlock and local client closure observations.
    pub release: Option<Arc<ProbeLeaseRelease>>,
}

impl std::fmt::Debug for WorkerSettlement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkerSettlement")
            .field("preparation_failed", &self.preparation.is_some())
            .field("driver_failed", &self.driver.is_some())
            .field("release_observed", &self.release.is_some())
            .finish()
    }
}

impl WorkerSettlement {
    /// Whether all recorded outcomes succeeded; a snapshot is not a completion wait.
    pub fn is_success(&self) -> bool {
        self.preparation.is_none()
            && self.driver.is_none()
            && self
                .release
                .as_ref()
                .is_none_or(|release| release.is_success())
    }
}

#[derive(Debug)]
struct TerminationFacts {
    state: AtomicU8,
    observed: watch::Sender<bool>,
    preparation_used: AtomicBool,
    preparing: watch::Sender<bool>,
    settlement: Mutex<WorkerSettlement>,
}

/// Cloneable native termination proof and independently retained worker outcomes.
///
/// ```
/// use batter_example_reference_service::worker::TerminationGate;
/// let gate = TerminationGate::new();
/// assert!(gate.settlement().is_success()); // Snapshot only; no work was started.
/// ```
#[derive(Clone, Debug)]
pub struct TerminationGate(Arc<TerminationFacts>);

impl Default for TerminationGate {
    fn default() -> Self {
        Self::new()
    }
}

impl TerminationGate {
    /// Construct a gate before preparation or native construction can start.
    #[must_use]
    pub fn new() -> Self {
        Self(Arc::new(TerminationFacts {
            state: AtomicU8::new(0),
            observed: watch::channel(false).0,
            preparation_used: AtomicBool::new(false),
            preparing: watch::channel(false).0,
            settlement: Mutex::new(WorkerSettlement::default()),
        }))
    }

    /// Return the strongest fact established about native task termination.
    pub fn state(&self) -> TerminationState {
        match self.0.state.load(Ordering::Acquire) {
            0 => TerminationState::NotStarted,
            1 => TerminationState::Unproven,
            2 => TerminationState::CooperativelyStopped,
            _ => unreachable!("only declared termination states are stored"),
        }
    }

    /// Inspect retained failures and release observations without waiting.
    pub fn settlement(&self) -> WorkerSettlement {
        self.0
            .settlement
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub(super) fn begin_preparation(&self) -> Result<(), WorkerBuildError> {
        self.0
            .preparation_used
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| WorkerBuildError::GateAlreadyUsed)?;
        if self.state() != TerminationState::NotStarted {
            return Err(WorkerBuildError::GateAlreadyUsed);
        }
        self.0.preparing.send_replace(true);
        Ok(())
    }

    pub(super) fn finish_preparation(&self, error: Option<Arc<WorkerStartupError>>) {
        self.0
            .settlement
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .preparation = error;
        self.0.preparing.send_replace(false);
    }

    pub(super) fn record_release(&self, release: Arc<ProbeLeaseRelease>) {
        self.0
            .settlement
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .release = Some(release);
    }

    pub(super) fn record_driver(&self, error: Arc<WorkerDriverError>) {
        self.0
            .settlement
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .driver = Some(error);
    }

    pub(super) fn arm(&self) -> Result<(), WorkerBuildError> {
        self.0
            .state
            .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| ())
            .map_err(|_| WorkerBuildError::GateAlreadyUsed)
    }
    pub(super) fn restore_not_started(&self) {
        self.0.state.store(0, Ordering::Release);
    }
    pub(super) fn mark_cooperatively_stopped(&self) {
        self.0.state.store(2, Ordering::Release);
        self.mark_driver_observed();
    }
    pub(super) fn mark_driver_observed(&self) {
        self.0.observed.send_replace(true);
    }

    pub(super) async fn wait_for_driver_observation(&self) {
        let mut preparing = self.0.preparing.subscribe();
        while *preparing.borrow_and_update() {
            preparing
                .changed()
                .await
                .expect("gate retains preparation publication");
        }
        if self.state() == TerminationState::NotStarted {
            return;
        }
        let mut observed = self.0.observed.subscribe();
        while !*observed.borrow_and_update() {
            observed
                .changed()
                .await
                .expect("gate retains driver publication");
        }
    }
}
