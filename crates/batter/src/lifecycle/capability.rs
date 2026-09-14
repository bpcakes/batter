use super::state::{PendingComponentStart, Shared};
use std::sync::Arc;
use tokio::time::Instant;

/// Process admission state, not an automatic dependency-health assessment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Readiness {
    /// Composition/root checks have not declared the process ready.
    Starting,
    /// Application explicitly enabled admission.
    Ready,
    /// Shutdown requested; stop admitting new work.
    Draining,
    /// The coordinator finished. This state is irreversible; check its report
    /// for incomplete termination.
    Stopped,
}

/// Root shutdown control; clones refer to the same process lifecycle.
/// Completion observation requires an owned driver: call
/// [`super::RunningSupervisor::observer`] after [`super::Supervisor::start`]. A
/// control handle can exist without a driver, so it cannot construct a completion
/// observer. Give readiness consumers [`LifecycleStatus`] and operation
/// entrypoints [`OperationAdmission`] instead of cloning this root authority.
///
/// ```compile_fail,E0599
/// use batter::lifecycle::ShutdownHandle;
///
/// let handle = ShutdownHandle::new();
/// let observer = handle.observer();
/// ```
///
/// ```compile_fail,E0599
/// use batter::lifecycle::ShutdownHandle;
///
/// let control = ShutdownHandle::new();
/// let readiness = control.readiness();
/// ```
///
/// ```compile_fail,E0599
/// use batter::lifecycle::ShutdownHandle;
///
/// let control = ShutdownHandle::new();
/// let token = control.operation_token();
/// ```
#[derive(Clone)]
pub struct ShutdownHandle {
    shared: Arc<Shared>,
}

impl Default for ShutdownHandle {
    fn default() -> Self {
        Self {
            shared: Arc::new(Shared::new(false)),
        }
    }
}

impl ShutdownHandle {
    /// An independent, initially unready lifecycle.
    pub fn new() -> Self {
        Self::default()
    }

    /// Declare application initialization complete. For a supervisor this arms
    /// readiness: it remains Starting until every component consumes its
    /// [`ComponentStartup`] acknowledgement. Returns true only for the first
    /// accepted declaration; never revives a draining/stopped service.
    pub fn mark_ready(&self) -> bool {
        self.shared.mark_ready()
    }

    /// Atomically withdraw readiness and signal drain. Does not immediately
    /// cancel admitted operation contexts.
    pub fn request(&self) {
        self.shared.request();
    }

    /// Project read-only readiness and lifecycle status.
    pub fn status(&self) -> LifecycleStatus {
        LifecycleStatus {
            shared: self.shared.clone(),
        }
    }

    /// Project readiness-gated, downward-cancelled operation admission.
    pub fn operation_admission(&self) -> OperationAdmission {
        OperationAdmission {
            shared: self.shared.clone(),
        }
    }

    /// Create standalone read-only shutdown observation.
    ///
    /// This neither registers nor acknowledges a component. A direct registered
    /// component must obtain its running observer by consuming the
    /// [`ComponentStartup`] supplied to its factory.
    pub fn signal(&self) -> ShutdownSignal {
        ShutdownSignal {
            shared: self.shared.clone(),
        }
    }
}

/// Read-only process readiness and lifecycle status.
///
/// This projection cannot approve readiness, request shutdown, admit operations,
/// or observe component cancellation. Use it for probes, status publication, and
/// readiness waiters.
///
/// ```
/// use batter::lifecycle::{Readiness, ShutdownHandle};
///
/// let control = ShutdownHandle::new();
/// let status = control.status();
/// assert_eq!(status.readiness(), Readiness::Starting);
/// ```
///
/// ```compile_fail,E0599
/// use batter::lifecycle::ShutdownHandle;
///
/// let status = ShutdownHandle::new().status();
/// status.request();
/// ```
///
/// ```compile_fail,E0599
/// use batter::lifecycle::ShutdownHandle;
///
/// let status = ShutdownHandle::new().status();
/// status.mark_ready();
/// ```
///
/// ```compile_fail,E0599
/// use batter::lifecycle::ShutdownHandle;
///
/// let status = ShutdownHandle::new().status();
/// let token = status.operation_token();
/// ```
#[derive(Clone)]
pub struct LifecycleStatus {
    shared: Arc<Shared>,
}

impl LifecycleStatus {
    /// Current process readiness state.
    pub fn readiness(&self) -> Readiness {
        self.shared.readiness()
    }

    /// Whether admission has closed and drain or stop has begun.
    pub fn is_draining(&self) -> bool {
        matches!(self.readiness(), Readiness::Draining | Readiness::Stopped)
    }

    /// Wait until readiness is acknowledged, or return the drain/stopped state
    /// if startup cannot become ready. Cancelling this waiter changes no state.
    /// Dropping an unstarted supervisor wakes this waiter with `Draining`.
    pub async fn wait_ready(&self) -> Result<(), Readiness> {
        self.shared.wait_ready().await
    }
}

/// Readiness-gated admission for transient operations.
///
/// A successful admission returns an [`crate::operation::OperationContext`]
/// whose cancellation travels downward from process forced cancellation. The
/// context cannot request process shutdown, and this capability cannot approve
/// readiness. A readiness read is the admission point: drain racing after that
/// read does not cancel the admitted context until forced cancellation.
///
/// ```no_run
/// use batter::lifecycle::ShutdownHandle;
/// use tokio::time::{Duration, Instant};
///
/// # fn example() -> Result<(), batter::lifecycle::Readiness> {
/// let control = ShutdownHandle::new();
/// control.mark_ready();
/// let admission = control.operation_admission();
/// let context = admission.admit(Instant::now() + Duration::from_secs(1))?;
/// # let _ = context;
/// # Ok(())
/// # }
/// ```
///
/// ```compile_fail,E0599
/// use batter::lifecycle::ShutdownHandle;
///
/// let admission = ShutdownHandle::new().operation_admission();
/// admission.request();
/// ```
///
/// ```compile_fail,E0599
/// use batter::lifecycle::ShutdownHandle;
///
/// let admission = ShutdownHandle::new().operation_admission();
/// admission.mark_ready();
/// ```
///
/// ```compile_fail,E0599
/// use batter::lifecycle::ShutdownHandle;
///
/// let admission = ShutdownHandle::new().operation_admission();
/// let token = admission.operation_token();
/// ```
#[derive(Clone)]
pub struct OperationAdmission {
    shared: Arc<Shared>,
}

impl OperationAdmission {
    /// Admit one operation only while the process is Ready.
    ///
    /// A past deadline is accepted and will fail at the operation boundary, as
    /// with [`crate::operation::OperationContext::at`]. The returned error is the
    /// observed non-ready lifecycle state and carries no application cause.
    pub fn admit(
        &self,
        deadline: Instant,
    ) -> Result<crate::operation::OperationContext, Readiness> {
        let readiness = self.shared.readiness();
        if readiness != Readiness::Ready {
            return Err(readiness);
        }
        let parent = self.shared.operation_token();
        Ok(crate::operation::OperationContext::under(deadline, &parent))
    }
}

/// Read-only drain and forced-cancellation observation.
///
/// This value carries no component-start or application-readiness authority.
/// Registered components receive [`ComponentStartup`] and consume its one
/// acknowledgement before entering their running phase.
///
/// ```compile_fail,E0599
/// use batter::lifecycle::ShutdownHandle;
///
/// let shutdown = ShutdownHandle::new().signal();
/// shutdown.acknowledge_started();
/// ```
///
/// ```compile_fail,E0599
/// use batter::lifecycle::ShutdownHandle;
///
/// let shutdown = ShutdownHandle::new().signal();
/// shutdown.request();
/// ```
///
/// ```compile_fail,E0599
/// use batter::lifecycle::ShutdownHandle;
///
/// let shutdown = ShutdownHandle::new().signal();
/// shutdown.mark_ready();
/// ```
///
/// ```compile_fail,E0599
/// use batter::lifecycle::ShutdownHandle;
///
/// let shutdown = ShutdownHandle::new().signal();
/// let token = shutdown.operation_token();
/// ```
#[derive(Clone)]
pub struct ShutdownSignal {
    shared: Arc<Shared>,
}

impl ShutdownSignal {
    /// Wait until admission closes and draining begins.
    pub async fn draining(&self) {
        self.shared.draining().await;
    }

    /// Wait for cooperative cancellation after the drain allowance is exhausted.
    pub async fn cancelled(&self) {
        self.shared.cancelled().await;
    }

    /// Whether drain has already been requested.
    pub fn is_draining(&self) -> bool {
        matches!(
            self.shared.readiness(),
            Readiness::Draining | Readiness::Stopped
        )
    }

    /// Whether forced cooperative cancellation has already been requested.
    pub fn is_cancelled(&self) -> bool {
        self.shared.is_cancelled()
    }
}

/// Private ownership of coordinator-only transitions and clocks.
#[derive(Clone)]
pub(super) struct LifecycleCoordinator {
    pub(super) shared: Arc<Shared>,
}

impl LifecycleCoordinator {
    pub(super) fn new(supervised: bool) -> Self {
        Self {
            shared: Arc::new(Shared::new(supervised)),
        }
    }

    pub(super) fn shutdown_handle(&self) -> ShutdownHandle {
        ShutdownHandle {
            shared: self.shared.clone(),
        }
    }

    pub(super) fn signal(&self) -> ShutdownSignal {
        ShutdownSignal {
            shared: self.shared.clone(),
        }
    }

    pub(super) fn is_draining(&self) -> bool {
        matches!(
            self.shared.readiness(),
            Readiness::Draining | Readiness::Stopped
        )
    }
}

/// The one pending startup obligation of a registered critical component.
///
/// Observe shutdown during initialization through [`Self::shutdown`]. After the
/// listener, worker, or other component is actually usable, consume this value
/// with [`Self::acknowledge_started`]. The returned [`ShutdownSignal`] carries
/// only the observation authority needed by the running component.
///
/// ```no_run
/// use batter::{BoxError, lifecycle::ComponentStartup};
///
/// async fn run_component(startup: ComponentStartup) -> Result<(), BoxError> {
///     if startup.shutdown().is_draining() {
///         return Ok(());
///     }
///     let shutdown = startup.acknowledge_started();
///     shutdown.draining().await;
///     Ok(())
/// }
/// ```
///
/// The capability is intentionally non-cloneable, and acknowledgement consumes
/// it so safe Rust cannot acknowledge the same registration twice:
///
/// ```compile_fail,E0599
/// use batter::lifecycle::ComponentStartup;
/// fn cannot_clone(startup: ComponentStartup) {
///     let duplicate = startup.clone();
/// }
/// ```
///
/// ```compile_fail,E0382
/// use batter::lifecycle::ComponentStartup;
/// fn cannot_acknowledge_twice(startup: ComponentStartup) {
///     let shutdown = startup.acknowledge_started();
///     let again = startup.acknowledge_started();
/// }
/// ```
pub struct ComponentStartup {
    shutdown: ShutdownSignal,
    pending: PendingComponentStart,
}

impl ComponentStartup {
    pub(super) fn registered(coordinator: &LifecycleCoordinator) -> Self {
        Self {
            shutdown: coordinator.signal(),
            pending: coordinator.shared.register_component(),
        }
    }

    /// Borrow read-only shutdown observation while initialization is pending.
    pub fn shutdown(&self) -> &ShutdownSignal {
        &self.shutdown
    }

    /// Record actual component initialization and enter the running phase.
    ///
    /// Acknowledgement during drain remains recorded but cannot revive readiness.
    /// Dropping this value without calling this method leaves startup pending.
    #[must_use = "retain the returned shutdown signal for the running component"]
    pub fn acknowledge_started(self) -> ShutdownSignal {
        let Self { shutdown, pending } = self;
        pending.acknowledge();
        shutdown
    }

    pub(super) fn into_parts(self) -> (ShutdownSignal, PendingComponentStart) {
        (self.shutdown, self.pending)
    }
}
