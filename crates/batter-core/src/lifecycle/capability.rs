use super::state::{PendingComponentStart, Shared};
use std::sync::Arc;

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
/// observer or application-readiness approval. Give readiness consumers
/// [`LifecycleStatus`] and operation entrypoints [`OperationAdmission`] instead
/// of cloning this root authority.
///
/// ```compile_fail,E0599
/// use batter_core::lifecycle::ShutdownHandle;
///
/// let handle = ShutdownHandle::new_unapproved();
/// let observer = handle.observer();
/// ```
///
/// ```compile_fail,E0599
/// use batter_core::lifecycle::ShutdownHandle;
///
/// let control = ShutdownHandle::new_unapproved();
/// let readiness = control.readiness();
/// ```
///
/// ```compile_fail,E0599
/// use batter_core::lifecycle::ShutdownHandle;
///
/// let control = ShutdownHandle::new_unapproved();
/// let token = control.operation_token();
/// ```
///
/// ```compile_fail,E0599
/// use batter_core::lifecycle::ShutdownHandle;
///
/// let control = ShutdownHandle::new_unapproved();
/// control.mark_ready();
/// ```
#[derive(Clone)]
pub struct ShutdownHandle {
    shared: Arc<Shared>,
}

impl ShutdownHandle {
    /// Create an independent lifecycle that deliberately has no readiness
    /// approval capability and therefore remains Starting until shutdown.
    ///
    /// This is for cancellation-only composition and tests. Use
    /// [`Self::new_with_readiness_approval`] whenever admission may become Ready.
    pub fn new_unapproved() -> Self {
        Self {
            shared: Arc::new(Shared::new(false)),
        }
    }

    /// Create an independent lifecycle together with its one application-
    /// readiness decision.
    ///
    /// This lower-level composition path has no owned process driver. Prefer
    /// [`super::Supervisor::start`] for supervised work; it keeps approval
    /// paired inside [`super::UnapprovedSupervisor`].
    ///
    /// ```
    /// use batter_core::lifecycle::{Readiness, ShutdownHandle};
    ///
    /// let (control, approval) = ShutdownHandle::new_with_readiness_approval();
    /// approval.approve();
    /// assert_eq!(control.status().readiness(), Readiness::Ready);
    /// ```
    pub fn new_with_readiness_approval() -> (Self, ReadinessApproval) {
        let (coordinator, approval) = LifecycleCoordinator::new(false);
        (coordinator.shutdown_handle(), approval)
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
/// use batter_core::lifecycle::{Readiness, ShutdownHandle};
///
/// let control = ShutdownHandle::new_unapproved();
/// let status = control.status();
/// assert_eq!(status.readiness(), Readiness::Starting);
/// ```
///
/// ```compile_fail,E0599
/// use batter_core::lifecycle::ShutdownHandle;
///
/// let status = ShutdownHandle::new_unapproved().status();
/// status.request();
/// ```
///
/// ```compile_fail,E0599
/// use batter_core::lifecycle::ShutdownHandle;
///
/// let status = ShutdownHandle::new_unapproved().status();
/// status.mark_ready();
/// ```
///
/// ```compile_fail,E0599
/// use batter_core::lifecycle::ShutdownHandle;
///
/// let status = ShutdownHandle::new_unapproved().status();
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
/// ```
/// use batter_core::lifecycle::ShutdownHandle;
/// use tokio::time::{Duration, Instant};
///
/// let (control, approval) = ShutdownHandle::new_with_readiness_approval();
/// approval.approve();
/// let admission = control.operation_admission();
/// let owner = admission
///     .admit_root(batter_core::operation::RootDeadline::at(Instant::now() + Duration::from_secs(1)))
///     .expect("application approval admits the operation");
/// assert!(owner.context().check().is_ok());
/// ```
///
/// ```compile_fail,E0599
/// use batter_core::lifecycle::ShutdownHandle;
///
/// let admission = ShutdownHandle::new_unapproved().operation_admission();
/// admission.request();
/// ```
///
/// ```compile_fail,E0599
/// use batter_core::lifecycle::ShutdownHandle;
///
/// let admission = ShutdownHandle::new_unapproved().operation_admission();
/// admission.mark_ready();
/// ```
///
/// ```compile_fail,E0599
/// use batter_core::lifecycle::ShutdownHandle;
///
/// let admission = ShutdownHandle::new_unapproved().operation_admission();
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
    /// with [`crate::operation::RootDeadline::at`]. The returned error is the
    /// observed non-ready lifecycle state and carries no application cause.
    pub fn admit_root(
        &self,
        deadline: crate::operation::RootDeadline,
    ) -> Result<crate::operation::OperationOwner, Readiness> {
        let readiness = self.shared.readiness();
        if readiness != Readiness::Ready {
            return Err(readiness);
        }
        let parent = self.shared.operation_token();
        Ok(crate::operation::OperationOwner::under(deadline, &parent))
    }
}

/// Read-only drain and forced-cancellation observation.
///
/// This value carries no component-start or application-readiness authority.
/// Registered components receive [`ComponentStartup`] and consume its one
/// acknowledgement before entering their running phase.
///
/// ```compile_fail,E0599
/// use batter_core::lifecycle::ShutdownHandle;
///
/// let shutdown = ShutdownHandle::new_unapproved().signal();
/// shutdown.acknowledge_started();
/// ```
///
/// ```compile_fail,E0599
/// use batter_core::lifecycle::ShutdownHandle;
///
/// let shutdown = ShutdownHandle::new_unapproved().signal();
/// shutdown.request();
/// ```
///
/// ```compile_fail,E0599
/// use batter_core::lifecycle::ShutdownHandle;
///
/// let shutdown = ShutdownHandle::new_unapproved().signal();
/// shutdown.mark_ready();
/// ```
///
/// ```compile_fail,E0599
/// use batter_core::lifecycle::ShutdownHandle;
///
/// let shutdown = ShutdownHandle::new_unapproved().signal();
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
    pub(super) fn new(supervised: bool) -> (Self, ReadinessApproval) {
        let shared = Arc::new(Shared::new(supervised));
        (
            Self {
                shared: shared.clone(),
            },
            ReadinessApproval { shared },
        )
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

/// One application-readiness decision for a lifecycle.
///
/// This capability is non-cloneable and [`Self::approve`] consumes it, so safe
/// Rust cannot approve the same decision twice. Ordinary supervised composition
/// receives it paired inside [`super::UnapprovedSupervisor`]; the standalone
/// constructor on [`ShutdownHandle`] returns it explicitly for low-level
/// integrations without an owned driver.
///
/// ```compile_fail,E0599
/// use batter_core::lifecycle::ReadinessApproval;
/// fn cannot_clone(approval: ReadinessApproval) {
///     let duplicate = approval.clone();
/// }
/// ```
///
/// ```compile_fail,E0382
/// use batter_core::lifecycle::ReadinessApproval;
/// fn cannot_approve_twice(approval: ReadinessApproval) {
///     approval.approve();
///     approval.approve();
/// }
/// ```
#[must_use = "consume the approval decision or explicitly leave the lifecycle unready"]
pub struct ReadinessApproval {
    shared: Arc<Shared>,
}

impl ReadinessApproval {
    /// Record application initialization approval. Pending component startup
    /// can still keep the process Starting; drain and stop remain irreversible.
    pub fn approve(self) {
        let _accepted = self.shared.mark_ready();
    }
}

/// Proof that a registered component resolved its startup obligation.
///
/// Component futures return `Result<ComponentExit, BoxError>`. The value is
/// produced only by [`RunningComponent::stopped`] after acknowledgement or by
/// [`ComponentStartup::abandon`] before it, so a component that runs to
/// completion without doing either cannot type-check. It carries no data and
/// cannot be constructed or cloned by applications.
///
/// ```compile_fail,E0423
/// use batter_core::lifecycle::ComponentExit;
/// let forged = ComponentExit(());
/// ```
#[derive(Debug)]
#[must_use = "return the exit proof from the component future"]
pub struct ComponentExit(());

impl ComponentExit {
    /// Exit proof for library-owned managed components.
    pub(super) fn managed() -> Self {
        Self(())
    }
}

/// The running phase of an acknowledged component.
///
/// It offers the component's drain and cancellation observation directly, and
/// [`Self::stopped`] is the only way to produce the [`ComponentExit`] proof
/// after acknowledgement. Lower-level drivers that take a plain signal can
/// clone one with [`Self::signal`]; the proof stays with this value, which is
/// neither cloneable nor reusable after it has been consumed.
///
/// ```compile_fail,E0599
/// use batter_core::lifecycle::RunningComponent;
/// fn cannot_clone(running: RunningComponent) {
///     let duplicate = running.clone();
/// }
/// ```
///
/// ```compile_fail,E0382
/// use batter_core::lifecycle::RunningComponent;
/// fn cannot_stop_twice(running: RunningComponent) {
///     let first = running.stopped();
///     let second = running.stopped();
/// }
/// ```
#[must_use = "retain the running component through its work and return stopped() from the component future"]
pub struct RunningComponent {
    shutdown: ShutdownSignal,
}

impl RunningComponent {
    /// Clone read-only shutdown observation for a lower-level driver.
    pub fn signal(&self) -> ShutdownSignal {
        self.shutdown.clone()
    }

    /// Wait until admission closes and draining begins.
    pub async fn draining(&self) {
        self.shutdown.draining().await;
    }

    /// Wait for cooperative cancellation after the drain allowance is exhausted.
    pub async fn cancelled(&self) {
        self.shutdown.cancelled().await;
    }

    /// Whether drain has already been requested.
    pub fn is_draining(&self) -> bool {
        self.shutdown.is_draining()
    }

    /// Whether forced cooperative cancellation has already been requested.
    pub fn is_cancelled(&self) -> bool {
        self.shutdown.is_cancelled()
    }

    /// Produce the exit proof once the component's running work has finished.
    ///
    /// Returning it before drain was requested is an unexpected exit and
    /// initiates shutdown, exactly like any other early success.
    pub fn stopped(self) -> ComponentExit {
        ComponentExit(())
    }
}

/// The one pending startup obligation of a registered critical component.
///
/// Observe shutdown during initialization through [`Self::shutdown`]. After the
/// listener, worker, or other component is actually usable, consume this value
/// with [`Self::acknowledge_started`]. The returned [`RunningComponent`] carries
/// the observation authority needed by the running component and is the only
/// source of the [`ComponentExit`] proof a component future must return after
/// acknowledging. A component that observes drain before it is usable exits
/// with [`Self::abandon`] instead. A component future therefore cannot return
/// success without either acknowledging or explicitly abandoning its startup.
///
/// ```no_run
/// use batter_core::{BoxError, lifecycle::{ComponentExit, ComponentStartup}};
///
/// async fn run_component(startup: ComponentStartup) -> Result<ComponentExit, BoxError> {
///     if startup.shutdown().is_draining() {
///         return Ok(startup.abandon());
///     }
///     let running = startup.acknowledge_started();
///     running.draining().await;
///     Ok(running.stopped())
/// }
/// ```
///
/// A component that neither acknowledges nor abandons cannot produce the proof:
///
/// ```compile_fail,E0308
/// use batter_core::{BoxError, lifecycle::{ComponentExit, ComponentStartup}};
/// async fn never_acknowledges(startup: ComponentStartup) -> Result<ComponentExit, BoxError> {
///     startup.shutdown().draining().await;
///     Ok(())
/// }
/// ```
///
/// The capability is intentionally non-cloneable, and acknowledgement consumes
/// it so safe Rust cannot acknowledge the same registration twice:
///
/// ```compile_fail,E0599
/// use batter_core::lifecycle::ComponentStartup;
/// fn cannot_clone(startup: ComponentStartup) {
///     let duplicate = startup.clone();
/// }
/// ```
///
/// ```compile_fail,E0382
/// use batter_core::lifecycle::ComponentStartup;
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
    /// Dropping this value without calling this method or [`Self::abandon`]
    /// leaves startup pending, and the component future then has no exit proof.
    pub fn acknowledge_started(self) -> RunningComponent {
        let Self { shutdown, pending } = self;
        pending.acknowledge();
        RunningComponent { shutdown }
    }

    /// Exit without initializing, typically because drain was observed first.
    ///
    /// The readiness gate stays pending, so the process cannot become Ready
    /// without this component. Returning the proof before drain was requested
    /// is an unexpected exit and initiates shutdown.
    pub fn abandon(self) -> ComponentExit {
        ComponentExit(())
    }

    pub(super) fn into_parts(self) -> (ShutdownSignal, PendingComponentStart) {
        (self.shutdown, self.pending)
    }
}
