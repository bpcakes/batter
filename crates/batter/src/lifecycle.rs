//! Process-owned critical tasks, two-stage shutdown, and explicit finalizers.
//!
//! Registered components are critical; dynamically admitted process tasks are
//! finite. Task-level errors initiate shutdown; ordinary business rejection can
//! remain a typed successful task value. No automatic restart is provided.

mod driver;
mod managed;
mod process;
mod report;
mod state;
mod tasks;
mod unix;

use state::Shared;
use tasks::TaskSet;

pub use driver::{
    DriverOutcome, RunningSupervisor, SharedShutdownReport, ShutdownFailure, SupervisorObserver,
    check_shutdown,
};
pub use process::{
    ProcessAdmissionError, ProcessHandle, ProcessReceipt, ProcessScope, ProcessTaskError,
};

pub use managed::{
    ManagedComponent, ManagedFailure, ManagedInitialization, ManagedObserver, ManagedOutcome,
    ManagedRecord, ManagedSettlement, ManagedShutdownBudget, SettlementEvidence,
};
pub use report::ShutdownReport;
pub(crate) use unix::install_reserved_signals;
pub use unix::{InstalledSignals, SignalRegistrationError, install_signals, register_signals};

use crate::{
    BoxError, ConfigurationError, RegistrationError,
    cleanup::{CleanupBudget, CleanupStack, SkipReason},
    scoped_dispatch, validation,
};
use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, atomic::AtomicBool},
    task::{Context, Poll},
    time::Duration,
};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

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

/// External lifecycle control; clones refer to the same process lifecycle.
/// Completion observation requires an owned driver: call
/// [`RunningSupervisor::observer`] after [`Supervisor::start`]. A control handle
/// can exist without a driver, so it cannot construct a completion observer.
///
/// ```compile_fail,E0599
/// use batter::lifecycle::ShutdownHandle;
///
/// let handle = ShutdownHandle::new();
/// let observer = handle.observer();
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
    /// readiness: it remains Starting until every component calls
    /// [`ShutdownSignal::mark_started`]. Returns true only for the first accepted
    /// declaration; never revives a draining/stopped service.
    pub fn mark_ready(&self) -> bool {
        self.shared.mark_ready()
    }

    /// Current admission state.
    pub fn readiness(&self) -> Readiness {
        self.shared.readiness()
    }

    /// Atomically withdraw readiness and signal drain. Does not immediately
    /// cancel admitted operation contexts.
    pub fn request(&self) {
        self.shared.request();
    }

    /// Whether draining has been requested.
    pub fn is_draining(&self) -> bool {
        matches!(self.readiness(), Readiness::Draining | Readiness::Stopped)
    }

    /// Wait for the stop-admission/drain signal.
    pub async fn draining(&self) {
        self.shared.draining().await;
    }

    /// Child token for admitted operations. Cancelling it cannot cancel the
    /// process. It is signalled at forced cancellation, not initial drain.
    pub fn operation_token(&self) -> CancellationToken {
        self.shared.operation_token()
    }

    /// Read-only signals for a managed component.
    pub fn signal(&self) -> ShutdownSignal {
        ShutdownSignal {
            handle: self.clone(),
            startup: None,
        }
    }

    /// Wait until readiness is acknowledged, or return the drain/stopped state
    /// if startup cannot become ready. Cancelling this waiter changes no state.
    /// Dropping an unstarted supervisor wakes this waiter with `Draining`.
    pub async fn wait_ready(&self) -> Result<(), Readiness> {
        self.shared.wait_ready().await
    }

    fn force_cancel(&self) {
        self.shared.force_cancel();
    }
}

/// Two distinct shutdown notifications for a managed component.
#[derive(Clone)]
pub struct ShutdownSignal {
    handle: ShutdownHandle,
    startup: Option<Arc<AtomicBool>>,
}

impl ShutdownSignal {
    /// Acknowledge that this registered component completed its initialization.
    /// Call only after the listener/worker is usable. Clones share one ack;
    /// returns false for repeated acknowledgements or an unregistered signal.
    pub fn mark_started(&self) -> bool {
        let Some(startup) = &self.startup else {
            return false;
        };
        self.handle.shared.mark_started(startup)
    }

    /// Stop accepting/claiming new work, then drain admitted work.
    pub async fn draining(&self) {
        self.handle.draining().await;
    }

    /// Cooperative interruption after the drain allowance is exhausted.
    pub async fn cancelled(&self) {
        self.handle.shared.cancelled().await;
    }

    /// Whether drain has already been requested.
    pub fn is_draining(&self) -> bool {
        self.handle.is_draining()
    }

    /// Whether forced cooperative cancellation has already been requested.
    pub fn is_cancelled(&self) -> bool {
        self.handle.shared.is_cancelled()
    }
}

/// Phase budgets, measured from one recorded stop time. Scheduling delays consume
/// that allowance; entering a later phase cannot restart its clock. These rely on
/// tasks yielding to the runtime. Cleanup has its own independent budget.
#[derive(Clone, Copy, Debug)]
pub struct ShutdownBudget {
    drain: Duration,
    cancel: Duration,
    abort_reap: Duration,
    cleanup: CleanupBudget,
}

impl ShutdownBudget {
    /// Drain/cancel may be zero; abort observation must be positive.
    pub fn new(
        drain: Duration,
        cancel: Duration,
        abort_reap: Duration,
        cleanup: CleanupBudget,
    ) -> Result<Self, ConfigurationError> {
        validation::representable(drain, "drain budget")?;
        validation::representable(cancel, "cancel budget")?;
        validation::positive(abort_reap, "abort reap budget")?;
        let value = drain
            .checked_add(cancel)
            .and_then(|v| v.checked_add(abort_reap))
            .and_then(|v| v.checked_add(cleanup.total_allowance()))
            .ok_or(ConfigurationError::TooLarge("shutdown combined budget"))?;
        validation::representable(value, "shutdown combined budget")?;
        Ok(Self {
            drain,
            cancel,
            abort_reap,
            cleanup,
        })
    }

    /// Sum of configured phase allowances, excluding scheduling/blocked-thread
    /// delays. This is not a hard wall-clock preemption guarantee.
    pub fn total_allowance(&self) -> Duration {
        self.drain + self.cancel + self.abort_reap + self.cleanup.total_allowance()
    }
}

/// Observed termination of a directly registered component.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskOutcome {
    /// A finite process task completed normally.
    Completed,
    /// Returned success after drain was requested.
    Stopped,
    /// Returned success before drain was requested.
    UnexpectedExit,
    /// Returned an application error.
    Failed,
    /// Panicked; JoinError is retained without logging its payload.
    Panicked,
    /// The task was aborted/cancelled by the runtime.
    Aborted,
}

/// A named task exit with the original error when available.
#[derive(Debug)]
pub struct TaskRecord {
    /// Registered component name or finite operation label. Finite labels may
    /// repeat for concurrent invocations; every failed invocation gets a record.
    pub name: &'static str,
    /// Observed termination category.
    pub outcome: TaskOutcome,
    /// Returned error or JoinError. May contain sensitive application data.
    pub error: Option<BoxError>,
}

impl TaskRecord {
    fn log_observation(&self) {
        let Self { name, outcome, .. } = self;
        if *outcome == TaskOutcome::Stopped {
            tracing::debug!(target: "batter", task = name, ?outcome, "task exit observed");
        } else {
            tracing::warn!(target: "batter", task = name, ?outcome, "task failure observed");
        }
    }
}

/// Trigger selected when the coordinator starts shutdown; ready requests take priority.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShutdownCause {
    /// External future or ShutdownHandle requested it.
    Requested,
    /// A registered critical component exited.
    ComponentExit(&'static str),
    /// An admitted finite task returned an error or panicked.
    /// Labels may repeat across invocations; success does not initiate shutdown.
    /// Shutdown aborts appear in [`ShutdownReport::abort_requested`] and task outcomes.
    /// See [`ProcessHandle::try_spawn`] for a finite-failure example.
    FiniteTaskExit(&'static str),
    /// No process work was registered; treated as a configuration failure.
    EmptySupervisor,
}

type ComponentFuture = Pin<Box<dyn Future<Output = Result<(), BoxError>> + Send + 'static>>;
struct Component {
    name: &'static str,
    factory: Box<dyn FnOnce(ShutdownSignal) -> ComponentFuture + Send + 'static>,
}

/// Own the process's critical tasks and dependency finalizers.
///
/// Registration is inert: factories start only when run_until is polled.
/// Application readiness must be declared explicitly through the handle.
/// Registered tasks must own and account for their own asynchronous children.
/// Dropping an unstarted supervisor signals drain and cancellation before
/// dropping its captures. It wakes readiness waiters but invokes no factories
/// or finalizers and publishes no completion report.
pub struct Supervisor {
    // First field: signal abandonment before dropping any application captures.
    // run_until transfers this guard into the outer driver future.
    ownership: Option<EmergencyShutdown>,
    components: Vec<Component>,
    managed: Vec<managed::Registration>,
    reserved_components: Vec<&'static str>,
    cleanup: CleanupStack,
    handle: ShutdownHandle,
    budget: ShutdownBudget,
    process: Option<ProcessHandle>,
    queued: Option<mpsc::Receiver<process::QueuedProcess>>,
}

impl Supervisor {
    /// Build an initially unready supervisor, without spawning anything.
    pub fn new(budget: ShutdownBudget) -> Self {
        let handle = ShutdownHandle {
            shared: Arc::new(Shared::new(true)),
        };
        Self {
            ownership: Some(EmergencyShutdown(handle.clone())),
            components: Vec::new(),
            managed: Vec::new(),
            reserved_components: Vec::new(),
            cleanup: CleanupStack::new(),
            handle,
            budget,
            process: None,
            queued: None,
        }
    }

    /// Enable bounded finite process work. Capacity covers queued plus running
    /// tasks; submissions reject immediately when full, never create waiters.
    pub fn with_process_capacity(
        budget: ShutdownBudget,
        capacity: usize,
    ) -> Result<Self, ConfigurationError> {
        let mut supervisor = Self::new(budget);
        let (process, queued) = ProcessHandle::new(supervisor.handle.clone(), capacity)?;
        supervisor.process = Some(process);
        supervisor.queued = Some(queued);
        Ok(supervisor)
    }

    /// Clone the finite-work submitting capability, if configured. It is inert
    /// before the driver starts and rejects root work before readiness.
    pub fn process_handle(&self) -> Option<ProcessHandle> {
        self.process.clone()
    }

    /// Shared readiness and shutdown control.
    pub fn handle(&self) -> ShutdownHandle {
        self.handle.clone()
    }

    /// Register long-lived, critical process work. Factories run inside their
    /// owned task, so factory panics are observed like task panics.
    pub fn register<F, Fut>(
        &mut self,
        name: &'static str,
        factory: F,
    ) -> Result<(), RegistrationError>
    where
        F: FnOnce(ShutdownSignal) -> Fut + Send + 'static,
        Fut: Future<Output = Result<(), BoxError>> + Send + 'static,
    {
        self.check_component_name(name)?;
        self.components.push(Component {
            name,
            factory: Box::new(move |signal| Box::pin(factory(signal)) as ComponentFuture),
        });
        self.handle.shared.register_component();
        Ok(())
    }

    fn check_component_name(&self, name: &'static str) -> Result<(), RegistrationError> {
        validation::name(name)?;
        if self.managed.iter().any(|component| component.name == name)
            || self.reserved_components.contains(&name)
            || self
                .components
                .iter()
                .any(|component| component.name == name)
        {
            return Err(RegistrationError::Duplicate(name));
        }
        Ok(())
    }

    pub(crate) fn reserve_component_name(
        &mut self,
        name: &'static str,
    ) -> Result<(), RegistrationError> {
        self.check_component_name(name)?;
        self.reserved_components.push(name);
        Ok(())
    }

    pub(crate) fn register_reserved<F, Fut>(&mut self, name: &'static str, factory: F)
    where
        F: FnOnce(ShutdownSignal) -> Fut + Send + 'static,
        Fut: Future<Output = Result<(), BoxError>> + Send + 'static,
    {
        let position = self
            .reserved_components
            .iter()
            .position(|reserved| *reserved == name)
            .expect("internal component reservation must exist exactly once");
        self.reserved_components.swap_remove(position);
        self.components.push(Component {
            name,
            factory: Box::new(move |signal| Box::pin(factory(signal)) as ComponentFuture),
        });
        self.handle.shared.register_component();
    }

    /// Register a native runtime through its adapter, retaining settlement even
    /// after its direct waiter is aborted. Name validation precedes any factory
    /// invocation; only the started driver invokes this synchronous factory.
    /// The absolute context bounds initialization, not native shutdown or cleanup.
    /// Application approval remains separate from the library-owned native ack.
    ///
    /// The factory must transfer all newly started native work in one
    /// [`ManagedComponent`]. Returning Err asserts that no native work was
    /// started; perform fallible validation before spawning. A factory panic
    /// makes termination uncertain and prevents dependency cleanup.
    /// Acquire asynchronous resources with
    /// [`crate::startup::Startup`] before registration. Prefer an adapter's
    /// registration function in application composition.
    pub fn register_managed<F, R>(
        &mut self,
        name: &'static str,
        context: crate::operation::OperationContext,
        factory: F,
    ) -> Result<(), RegistrationError>
    where
        F: FnOnce(ManagedShutdownBudget) -> Result<ManagedComponent<R>, BoxError> + Send + 'static,
        R: ManagedSettlement,
    {
        self.check_component_name(name)?;
        self.managed
            .push(managed::Registration::new(name, context, factory));
        self.handle.shared.register_component();
        Ok(())
    }

    /// Register owned resource teardown. Dependents must be registered AFTER
    /// dependencies, so reversing the order closes them safely.
    pub fn on_cleanup<F, Fut>(
        &mut self,
        name: &'static str,
        action: F,
    ) -> Result<(), RegistrationError>
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = Result<(), BoxError>> + Send + 'static,
    {
        self.cleanup.push(name, action)
    }

    /// Reserve a validated cleanup name before acquiring a resource.
    /// Registration through the returned slot cannot reject ownership afterward.
    /// See [`crate::cleanup::CleanupSlot`] for the acquisition pattern and limits.
    pub fn reserve_cleanup(
        &mut self,
        name: &'static str,
    ) -> Result<crate::cleanup::CleanupSlot<'_>, RegistrationError> {
        self.cleanup.reserve(name)
    }

    /// Extract pending finalizers if application startup fails before running.
    /// The application must explicitly close this stack and retain both errors.
    /// Dropping the supervisor cancels its operation tokens, even after this
    /// extraction. Finalizers must use cleanup independent of those tokens
    /// (for example, a fresh [`crate::operation::OperationContext::new`]), not
    /// a context derived from [`ShutdownHandle::operation_token`]. The stack's
    /// [`CleanupBudget`] still bounds explicit teardown.
    pub fn take_cleanup(&mut self) -> CleanupStack {
        std::mem::take(&mut self.cleanup)
    }

    /// Observe failures, stop admission, drain, cancel, abort/reap, then close
    /// dependencies. A panic, requested abort, or unjoined direct task causes
    /// conservative skipping of resource finalizers (not false success).
    ///
    /// Construction takes ownership immediately, while factories and readiness
    /// remain inert until polling. Dropping even a never-polled driver signals
    /// drain and cancellation. Once polled, JoinSet also requests abortion, but
    /// Drop cannot await children or finalizers. Use the normal shutdown protocol
    /// or [`Supervisor::start`] for awaited cleanup; a hard kill cannot run it.
    pub fn run_until<F>(mut self, shutdown: F) -> impl Future<Output = ShutdownReport>
    where
        F: Future<Output = ()>,
    {
        let emergency = self
            .ownership
            .take()
            .expect("supervisor owns shutdown signaling before driver transfer");
        CallerOwnedDriver {
            _ownership: emergency,
            future: async move { scoped_dispatch::scope(self.drive_until(shutdown)).await },
        }
    }

    async fn drive_until<F>(mut self, shutdown: F) -> ShutdownReport
    where
        F: Future<Output = ()>,
    {
        self.handle.shared.start_driver();
        let mut tasks = TaskSet::default();
        let mut managed = Vec::new();
        for registration in self.managed.drain(..) {
            let name = registration.name;
            let (component, observer) = registration.prepare(self.budget);
            managed.push((name, observer));
            tasks.spawn_component(component, &self.handle);
        }
        for component in self.components.drain(..) {
            tasks.spawn_component(component, &self.handle);
        }
        tokio::pin!(shutdown);
        let cause = self.wait_for_shutdown(shutdown.as_mut(), &mut tasks).await;
        self.handle.request();
        let drain = self.budget.drain;
        let cancel = drain + self.budget.cancel;
        let reap = cancel + self.budget.abort_reap;
        tracing::info!(target: "batter", "shutdown drain started");
        tasks
            .collect_until(&mut self.queued, &self.handle, drain)
            .await;
        let forced_cancellation = tasks.unfinished(&self.handle);
        self.handle.force_cancel();
        tasks
            .collect_until(&mut self.queued, &self.handle, cancel)
            .await;
        // No new descendants can arrive after forced cancellation. Transfer all
        // already admitted queue entries into the owned JoinSet before abort.
        if let Some(queued) = &mut self.queued {
            while let Ok(task) = queued.try_recv() {
                tasks.spawn_process(task);
            }
        }
        tasks.collect_ready(&self.handle);
        let abort_requested = tasks.abort_unfinished();
        if !tasks.is_empty() {
            tasks
                .collect_until(&mut self.queued, &self.handle, reap)
                .await;
        }
        let summary = tasks.finish();
        let managed_records = managed::freeze(managed, &self.handle, reap).await;
        // Joining a wrapper does not prove that its hidden children have ended.
        // Be conservative after panic or forced abort, particularly for servers.
        let unsafe_exit = !abort_requested.is_empty()
            || summary.unsafe_exit
            || managed_records
                .iter()
                .any(|record| !record.outcome.allows_dependency_cleanup());
        let cleanup = if unsafe_exit {
            self.cleanup.skip(SkipReason::UnsafeTaskExit)
        } else {
            self.cleanup.close(self.budget.cleanup).await
        };
        self.handle.shared.stop_driver();
        ShutdownReport {
            cause,
            tasks: summary.records,
            managed: managed_records,
            completed_process_tasks: summary.completed,
            forced_cancellation,
            abort_requested,
            unjoined: summary.unjoined,
            cleanup,
        }
    }

    async fn wait_for_shutdown<F>(&mut self, shutdown: F, tasks: &mut TaskSet) -> ShutdownCause
    where
        F: Future<Output = ()>,
    {
        tokio::pin!(shutdown);
        if tasks.is_empty() && self.process.is_none() {
            ShutdownCause::EmptySupervisor
        } else {
            loop {
                tokio::select! {
                    biased;
                    // Draining cannot be starved by a stream of finite success.
                    _ = self.handle.draining() => break ShutdownCause::Requested,
                    _ = &mut shutdown => break ShutdownCause::Requested,
                    cause = tasks.next_exit(&self.handle), if !tasks.is_empty() => {
                        if let Some(cause) = cause {
                            break cause;
                        }
                    }
                    Some(task) = receive_process(&mut self.queued) => tasks.spawn_process(task),
                }
            }
        }
    }
}

struct EmergencyShutdown(ShutdownHandle);
impl Drop for EmergencyShutdown {
    fn drop(&mut self) {
        self.0.force_cancel();
    }
}

pin_project_lite::pin_project! {
    // Struct field order makes signaling precede inner-future destruction,
    // including before the first poll; async capture drop order is not needed.
    struct CallerOwnedDriver<F> {
        _ownership: EmergencyShutdown,
        #[pin]
        future: F,
    }
}

impl<F: Future> Future for CallerOwnedDriver<F> {
    type Output = F::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.project().future.poll(cx)
    }
}

async fn receive_process(
    queued: &mut Option<mpsc::Receiver<process::QueuedProcess>>,
) -> Option<process::QueuedProcess> {
    match queued {
        Some(queued) => queued.recv().await,
        None => std::future::pending().await,
    }
}
