//! Process-owned critical tasks, two-stage shutdown, and explicit finalizers.
//!
//! Registered components are critical; dynamically admitted process tasks are
//! finite. A finite task initiates shutdown only by returning [`Fatal`]; ordinary
//! business rejection remains a typed successful task value and cannot be
//! propagated into a drain with `?`. No automatic restart is provided.

mod caller_owned;
mod capability;
mod driver;
mod managed;
mod process;
mod report;
mod state;
mod tasks;
mod unix;

use caller_owned::SupervisorOwnership;
use capability::LifecycleCoordinator;
use tasks::TaskSet;

pub use caller_owned::UnapprovedDriver;
pub use capability::{
    ComponentExit, ComponentStartup, LifecycleStatus, OperationAdmission, Readiness,
    ReadinessApproval, RunningComponent, ShutdownHandle, ShutdownSignal,
};
pub use driver::{
    DriverOutcome, RunningSupervisor, SharedShutdownReport, ShutdownFailure, ShutdownSuccess,
    SupervisorObserver, UnapprovedSupervisor, check_shutdown,
};
pub use process::{
    Fatal, ProcessAdmissionError, ProcessCapacity, ProcessHandle, ProcessReceipt, ProcessScope,
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
use std::{future::Future, pin::Pin, time::Duration};
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
/// Debug reports only the name, outcome and whether an error was retained.
pub struct TaskRecord {
    /// Registered component name or finite operation label. Finite labels may
    /// repeat for concurrent invocations; every failed invocation gets a record.
    pub name: &'static str,
    /// Observed termination category.
    pub outcome: TaskOutcome,
    /// Returned error or JoinError. May contain sensitive application data.
    pub error: Option<BoxError>,
}

impl std::fmt::Debug for TaskRecord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TaskRecord")
            .field("name", &self.name)
            .field("outcome", &self.outcome)
            .field("error", &self.error.as_ref().map(|_| "retained"))
            .finish()
    }
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
    /// An admitted finite task returned [`Fatal`] or panicked.
    /// Labels may repeat across invocations; success does not initiate shutdown.
    /// Shutdown aborts appear in [`ShutdownReport::abort_requested`] and task outcomes.
    /// See [`ProcessHandle::try_spawn`] for a finite-failure example.
    FiniteTaskExit(&'static str),
    /// No process work was registered; treated as a configuration failure.
    EmptySupervisor,
}

type ComponentFuture =
    Pin<Box<dyn Future<Output = Result<ComponentExit, BoxError>> + Send + 'static>>;
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
/// Registration is inert: factories start only when a driver is polled. Ordinary
/// `start`/`run_until` consume application approval once; explicitly named
/// unapproved variants retain the non-cloneable decision with the driver.
/// Registered tasks must own and account for their own asynchronous children.
/// Dropping an unstarted supervisor signals drain and cancellation before
/// dropping its captures. It wakes readiness waiters but invokes no factories
/// or finalizers and publishes no completion report.
pub struct Supervisor {
    // First field: signal abandonment before dropping any application captures.
    // Starting transfers this guard and the paired approval into the driver.
    ownership: Option<SupervisorOwnership>,
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
        let (coordinator, approval) = LifecycleCoordinator::new(true);
        Self {
            ownership: Some(SupervisorOwnership::new(coordinator.clone(), approval)),
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
    /// use batter_core::lifecycle::{ShutdownBudget, Supervisor};
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

    /// Clone root shutdown control.
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
    ///
    /// The component future must return a [`ComponentExit`] proof, obtainable
    /// only by acknowledging startup and then calling
    /// [`RunningComponent::stopped`], or by [`ComponentStartup::abandon`]
    /// before initialization. A component cannot complete without doing one.
    pub fn register<F, Fut>(
        &mut self,
        name: &'static str,
        factory: F,
    ) -> Result<(), RegistrationError>
    where
        F: FnOnce(ComponentStartup) -> Fut + Send + 'static,
        Fut: Future<Output = Result<ComponentExit, BoxError>> + Send + 'static,
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
        Fut: Future<Output = Result<ComponentExit, BoxError>> + Send + 'static,
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
    /// [`crate::operation::OperationOwner::new`]), not
    /// a context derived from [`OperationAdmission`]. The stack's
    /// [`CleanupBudget`] still bounds explicit teardown.
    pub fn take_cleanup(&mut self) -> CleanupStack {
        std::mem::take(&mut self.cleanup)
    }

    /// Approve application readiness once, then observe failures, stop admission,
    /// drain, cancel, abort/reap, and close dependencies. A panic, requested abort,
    /// or unjoined direct task causes conservative skipping of resource finalizers
    /// (not false success).
    ///
    /// Construction takes ownership immediately, while factories and readiness
    /// remain inert until polling. Dropping even a never-polled driver signals
    /// drain and cancellation. Once polled, JoinSet also requests abortion, but
    /// Drop cannot await children or finalizers. Use the normal shutdown protocol
    /// or [`Supervisor::start`] for awaited cleanup; a hard kill cannot run it.
    ///
    /// ```no_run
    /// use batter_core::{
    ///     BoxError,
    ///     cleanup::CleanupBudget,
    ///     lifecycle::{ShutdownBudget, Supervisor},
    /// };
    /// use std::time::Duration;
    ///
    /// # async fn example() -> Result<(), BoxError> {
    /// let second = Duration::from_secs(1);
    /// let cleanup = CleanupBudget::new(second, second, second)?;
    /// let mut supervisor = Supervisor::new(ShutdownBudget::new(
    ///     second, second, second, cleanup,
    /// )?);
    /// supervisor.register("worker", |startup| async move {
    ///     let shutdown = startup.acknowledge_started();
    ///     shutdown.draining().await;
    ///     Ok(shutdown.stopped())
    /// })?;
    /// let report = supervisor.run_until(std::future::ready(())).await;
    /// assert!(report.is_success());
    /// # Ok(()) }
    /// ```
    pub fn run_until<F>(self, shutdown: F) -> impl Future<Output = ShutdownReport>
    where
        F: Future<Output = ()>,
    {
        let (approval, driver) = self.into_unapproved_driver(shutdown);
        async move {
            approval.approve();
            driver.await
        }
    }

    /// Drive the caller-owned shutdown protocol without approving application
    /// readiness.
    ///
    /// This exceptional path keeps admission Starting while the returned
    /// [`UnapprovedDriver`] is polled. Prefer [`Self::run_until`] unless a separate
    /// policy stage deliberately withholds approval. The outer typestate remains
    /// movable after polling, so its consuming approval transition can happen at
    /// that later boundary without detaching authority from the driver. The
    /// future remains inert until polled, and consuming the supervisor prevents
    /// any second driver.
    ///
    /// ```no_run
    /// # use batter_core::{cleanup::CleanupBudget, lifecycle::{Readiness, ShutdownBudget, Supervisor}};
    /// # use std::{future::pending, time::Duration};
    /// # async fn example() -> Result<(), batter_core::BoxError> {
    /// # let second = Duration::from_secs(1);
    /// # let cleanup = CleanupBudget::new(second, second, second)?;
    /// let supervisor = Supervisor::new(ShutdownBudget::new(
    ///     second, second, second, cleanup,
    /// )?);
    /// let status = supervisor.status();
    /// let driver = supervisor.run_until_unapproved(pending());
    /// assert_eq!(status.readiness(), Readiness::Starting);
    /// let driver = driver.approve_readiness();
    /// drop(driver); // Abandonment still requests drain and cancellation.
    /// # Ok(()) }
    /// ```
    pub fn run_until_unapproved<F>(
        self,
        shutdown: F,
    ) -> UnapprovedDriver<impl Future<Output = ShutdownReport>>
    where
        F: Future<Output = ()>,
    {
        let (approval, driver) = self.into_unapproved_driver(shutdown);
        UnapprovedDriver::new(driver, approval)
    }

    fn into_unapproved_driver<F>(
        mut self,
        shutdown: F,
    ) -> (ReadinessApproval, impl Future<Output = ShutdownReport>)
    where
        F: Future<Output = ()>,
    {
        let ownership = self
            .ownership
            .take()
            .expect("supervisor owns shutdown signaling before driver transfer");
        ownership
            .into_driver(async move { scoped_dispatch::scope(self.drive_until(shutdown)).await })
    }

    async fn drive_until<F>(self, shutdown: F) -> ShutdownReport
    where
        F: Future<Output = ()>,
    {
        // Armed before any work so an abandoned or unwinding driver still
        // records exactly one shutdown. Declared before the drive future,
        // which owns the supervisor, so on abandonment the supervisor's queued
        // work and cleanup are destroyed (and recorded) first.
        let mut terminal = crate::telemetry::record::ShutdownTerminal::new();
        self.drive(shutdown, &mut terminal).await
    }

    async fn drive<F>(
        mut self,
        shutdown: F,
        terminal: &mut crate::telemetry::record::ShutdownTerminal,
    ) -> ShutdownReport
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
        // Measure from the canonical stop instant that every phase budget uses.
        let stop_started = self.coordinator.shared.stop_started();
        terminal.draining(
            cause,
            stop_started.unwrap_or_else(tokio::time::Instant::now),
        );
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
        let report = ShutdownReport {
            cause,
            tasks: summary.records,
            managed: managed_records,
            completed_process_tasks: summary.completed,
            forced_cancellation,
            abort_requested,
            unjoined: summary.unjoined,
            cleanup,
        };
        // Publishing Stopped never depends on recorder code. The sample is
        // recorded before the report is returned, so a root that flushes
        // after awaiting the driver's completion observes it.
        self.coordinator.shared.stop_driver();
        terminal.finish(report.is_success());
        report
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

async fn receive_process(
    queued: &mut Option<mpsc::Receiver<process::QueuedProcess>>,
) -> Option<process::QueuedProcess> {
    match queued {
        Some(queued) => queued.recv().await,
        None => std::future::pending().await,
    }
}
