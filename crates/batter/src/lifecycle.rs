//! Process-owned critical tasks, two-stage shutdown, and explicit finalizers.
//!
//! Registered components are critical; dynamically admitted process tasks are
//! finite. Task-level errors initiate shutdown; ordinary business rejection can
//! remain a typed successful task value. No automatic restart is provided.

mod capability;
mod driver;
mod managed;
mod process;
mod report;
mod state;
mod tasks;
mod unix;

use capability::LifecycleCoordinator;
use tasks::TaskSet;

pub use capability::{
    ComponentStartup, LifecycleStatus, OperationAdmission, Readiness, ShutdownHandle,
    ShutdownSignal,
};
pub use driver::{
    DriverOutcome, RunningSupervisor, SharedShutdownReport, ShutdownFailure, SupervisorObserver,
    check_shutdown,
};
pub use process::{
    ProcessAdmissionError, ProcessCapacity, ProcessHandle, ProcessReceipt, ProcessScope,
    ProcessTaskError,
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
    task::{Context, Poll},
    time::Duration,
};
use tokio::sync::mpsc;

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
struct RegisteredComponent {
    startup: ComponentStartup,
    coordinator: LifecycleCoordinator,
}

impl RegisteredComponent {
    fn new(coordinator: &LifecycleCoordinator) -> Self {
        Self {
            startup: ComponentStartup::registered(coordinator),
            coordinator: coordinator.clone(),
        }
    }
}

struct Component {
    name: &'static str,
    lifecycle: RegisteredComponent,
    factory:
        Box<dyn FnOnce(ComponentStartup, LifecycleCoordinator) -> ComponentFuture + Send + 'static>,
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
    coordinator: LifecycleCoordinator,
    budget: ShutdownBudget,
    process: Option<ProcessHandle>,
    queued: Option<mpsc::Receiver<process::QueuedProcess>>,
}

impl Supervisor {
    /// Build an initially unready supervisor, without spawning anything.
    pub fn new(budget: ShutdownBudget) -> Self {
        let coordinator = LifecycleCoordinator::new(true);
        Self {
            ownership: Some(EmergencyShutdown(coordinator.clone())),
            components: Vec::new(),
            managed: Vec::new(),
            reserved_components: Vec::new(),
            cleanup: CleanupStack::new(),
            coordinator,
            budget,
            process: None,
            queued: None,
        }
    }

    /// Enable bounded finite process work. Capacity covers queued plus running
    /// tasks; submissions reject immediately when full, never create waiters.
    ///
    /// ```compile_fail,E0308
    /// use batter::lifecycle::{ShutdownBudget, Supervisor};
    ///
    /// fn cannot_build_from_raw(budget: ShutdownBudget, capacity: usize) {
    ///     let supervisor = Supervisor::with_process_capacity(budget, capacity);
    /// }
    /// ```
    pub fn with_process_capacity(budget: ShutdownBudget, capacity: ProcessCapacity) -> Self {
        let mut supervisor = Self::new(budget);
        let (process, queued) = ProcessHandle::new(supervisor.coordinator.clone(), capacity);
        supervisor.process = Some(process);
        supervisor.queued = Some(queued);
        supervisor
    }

    /// Clone the finite-work submitting capability, if configured. It is inert
    /// before the driver starts and rejects root work before readiness.
    pub fn process_handle(&self) -> Option<ProcessHandle> {
        self.process.clone()
    }

    /// Clone root shutdown and application-readiness control.
    /// Prefer [`Self::status`] or [`Self::operation_admission`] for consumers
    /// that do not need those mutations.
    pub fn handle(&self) -> ShutdownHandle {
        self.coordinator.shutdown_handle()
    }

    /// Clone read-only readiness and lifecycle status.
    pub fn status(&self) -> LifecycleStatus {
        self.handle().status()
    }

    /// Clone readiness-gated admission for transient operation contexts.
    pub fn operation_admission(&self) -> OperationAdmission {
        self.handle().operation_admission()
    }

    /// Register long-lived, critical process work. Factories run inside their
    /// owned task, so factory panics are observed like task panics.
    pub fn register<F, Fut>(
        &mut self,
        name: &'static str,
        factory: F,
    ) -> Result<(), RegistrationError>
    where
        F: FnOnce(ComponentStartup) -> Fut + Send + 'static,
        Fut: Future<Output = Result<(), BoxError>> + Send + 'static,
    {
        self.check_component_name(name)?;
        let lifecycle = RegisteredComponent::new(&self.coordinator);
        self.components.push(Component {
            name,
            lifecycle,
            factory: Box::new(move |startup, _coordinator| {
                Box::pin(factory(startup)) as ComponentFuture
            }),
        });
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
        F: FnOnce(ComponentStartup) -> Fut + Send + 'static,
        Fut: Future<Output = Result<(), BoxError>> + Send + 'static,
    {
        let position = self
            .reserved_components
            .iter()
            .position(|reserved| *reserved == name)
            .expect("internal component reservation must exist exactly once");
        self.reserved_components.swap_remove(position);
        let lifecycle = RegisteredComponent::new(&self.coordinator);
        self.components.push(Component {
            name,
            lifecycle,
            factory: Box::new(move |startup, _coordinator| {
                Box::pin(factory(startup)) as ComponentFuture
            }),
        });
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
        let lifecycle = RegisteredComponent::new(&self.coordinator);
        self.managed.push(managed::Registration::new(
            name, lifecycle, context, factory,
        ));
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
    /// Dropping the supervisor signals forced process cancellation even after
    /// this extraction. Finalizers must use cleanup independent of admitted
    /// operation contexts (for example, a fresh
    /// [`crate::operation::OperationContext::new`]), not
    /// a context derived from [`OperationAdmission`]. The stack's
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
        self.coordinator.shared.start_driver();
        let mut tasks = TaskSet::default();
        let mut managed = Vec::new();
        for registration in self.managed.drain(..) {
            let name = registration.name;
            let (component, observer) = registration.prepare(self.budget);
            managed.push((name, observer));
            tasks.spawn_component(component);
        }
        for component in self.components.drain(..) {
            tasks.spawn_component(component);
        }
        tokio::pin!(shutdown);
        let cause = self.wait_for_shutdown(shutdown.as_mut(), &mut tasks).await;
        self.coordinator.shared.request();
        let drain = self.budget.drain;
        let cancel = drain + self.budget.cancel;
        let reap = cancel + self.budget.abort_reap;
        tracing::info!(target: "batter", "shutdown drain started");
        tasks
            .collect_until(&mut self.queued, &self.coordinator, drain)
            .await;
        let forced_cancellation = tasks.unfinished(&self.coordinator);
        self.coordinator.shared.force_cancel();
        tasks
            .collect_until(&mut self.queued, &self.coordinator, cancel)
            .await;
        // No new descendants can arrive after forced cancellation. Transfer all
        // already admitted queue entries into the owned JoinSet before abort.
        if let Some(queued) = &mut self.queued {
            while let Ok(task) = queued.try_recv() {
                tasks.spawn_process(task);
            }
        }
        tasks.collect_ready(&self.coordinator);
        let abort_requested = tasks.abort_unfinished();
        if !tasks.is_empty() {
            tasks
                .collect_until(&mut self.queued, &self.coordinator, reap)
                .await;
        }
        let summary = tasks.finish();
        let managed_records = managed::freeze(managed, &self.coordinator, reap).await;
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
        self.coordinator.shared.stop_driver();
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
                    _ = self.coordinator.shared.draining() => break ShutdownCause::Requested,
                    _ = &mut shutdown => break ShutdownCause::Requested,
                    cause = tasks.next_exit(&self.coordinator), if !tasks.is_empty() => {
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

struct EmergencyShutdown(LifecycleCoordinator);
impl Drop for EmergencyShutdown {
    fn drop(&mut self) {
        self.0.shared.force_cancel();
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
