//! Process-owned critical tasks, two-stage shutdown, and explicit finalizers.
//!
//! Registered components are critical; dynamically admitted process tasks are
//! finite. Task-level errors initiate shutdown; ordinary business rejection can
//! remain a typed successful task value. No automatic restart is provided.

mod driver;
mod process;

pub use driver::{DriverOutcome, RunningSupervisor, SupervisorObserver};
pub use process::{
    ProcessAdmissionError, ProcessHandle, ProcessReceipt, ProcessScope, ProcessTaskError,
};

use crate::{
    BoxError, ConfigurationError, RegistrationError,
    cleanup::{CleanupBudget, CleanupReport, CleanupStack, SkipReason},
    scoped_dispatch, validation,
};
use std::{
    collections::HashMap,
    future::Future,
    pin::Pin,
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, AtomicU8, Ordering},
    },
    time::Duration,
};
use tokio::{
    sync::{Notify, mpsc, watch},
    task::{AbortHandle, Id, JoinError, JoinSet},
    time::Instant,
};
use tokio_util::sync::CancellationToken;
use tracing::Instrument;

const STARTING: u8 = 0;
const READY: u8 = 1;
const DRAINING: u8 = 2;
const STOPPED: u8 = 3;

/// Process admission state, not an automatic dependency-health assessment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Readiness {
    /// Composition/root checks have not declared the process ready.
    Starting,
    /// Application explicitly enabled admission.
    Ready,
    /// Shutdown requested; stop admitting new work.
    Draining,
    /// The coordinator finished. Check its report for incomplete termination.
    Stopped,
}

struct Shared {
    state: AtomicU8,
    drain: CancellationToken,
    cancel: CancellationToken,
    admission: Mutex<AdmissionState>,
    changed: Notify,
    completion: watch::Sender<Option<DriverOutcome>>,
}

struct AdmissionState {
    supervised: bool,
    running: bool,
    ready_requested: bool,
    pending_startups: usize,
    finite_active: usize,
    forced: bool,
    failed: bool,
}

impl Shared {
    fn admission(&self) -> MutexGuard<'_, AdmissionState> {
        self.admission
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }

    fn publish_ready(&self, admission: &AdmissionState) {
        if admission.ready_requested
            && (!admission.supervised || admission.running)
            && admission.pending_startups == 0
        {
            let _ =
                self.state
                    .compare_exchange(STARTING, READY, Ordering::AcqRel, Ordering::Acquire);
            self.changed.notify_waiters();
        }
    }
}

/// External lifecycle control; clones refer to the same process lifecycle.
#[derive(Clone)]
pub struct ShutdownHandle {
    shared: Arc<Shared>,
}

impl Default for ShutdownHandle {
    fn default() -> Self {
        Self {
            shared: Arc::new(Shared {
                state: AtomicU8::new(STARTING),
                drain: CancellationToken::new(),
                cancel: CancellationToken::new(),
                admission: Mutex::new(AdmissionState {
                    supervised: false,
                    running: false,
                    ready_requested: false,
                    pending_startups: 0,
                    finite_active: 0,
                    forced: false,
                    failed: false,
                }),
                changed: Notify::new(),
                completion: watch::channel(None).0,
            }),
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
        let mut admission = self.shared.admission();
        if admission.ready_requested || self.shared.state.load(Ordering::Acquire) >= DRAINING {
            return false;
        }
        admission.ready_requested = true;
        self.shared.publish_ready(&admission);
        true
    }

    /// Current admission state.
    pub fn readiness(&self) -> Readiness {
        match self.shared.state.load(Ordering::Acquire) {
            STARTING => Readiness::Starting,
            READY => Readiness::Ready,
            DRAINING => Readiness::Draining,
            _ => Readiness::Stopped,
        }
    }

    /// Atomically withdraw readiness and signal drain. Does not immediately
    /// cancel admitted operation contexts.
    pub fn request(&self) {
        {
            // Root process admission uses this same lock: enqueue and drain are
            // ordered, including work accepted but not yet polled by the driver.
            let _admission = self.shared.admission();
            if self.shared.state.load(Ordering::Acquire) != STOPPED {
                self.shared.state.store(DRAINING, Ordering::Release);
            }
        }
        self.shared.drain.cancel();
        self.shared.changed.notify_waiters();
    }

    /// Whether draining has been requested.
    pub fn is_draining(&self) -> bool {
        self.shared.state.load(Ordering::Acquire) >= DRAINING
    }

    /// Wait for the stop-admission/drain signal.
    pub async fn draining(&self) {
        self.shared.drain.cancelled().await;
    }

    /// Child token for admitted operations. Cancelling it cannot cancel the
    /// process. It is signalled at forced cancellation, not initial drain.
    pub fn operation_token(&self) -> CancellationToken {
        self.shared.cancel.child_token()
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
    pub async fn wait_ready(&self) -> Result<(), Readiness> {
        loop {
            let notified = self.shared.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            match self.readiness() {
                Readiness::Ready => return Ok(()),
                Readiness::Starting => notified.await,
                state => return Err(state),
            }
        }
    }

    /// Observe the retained completion of an owned [`Supervisor::start`] driver.
    /// `run_until` does not publish into this observer.
    pub fn observer(&self) -> SupervisorObserver {
        SupervisorObserver::new(self.shared.completion.subscribe())
    }

    fn force_cancel(&self) {
        self.shared.admission().forced = true;
        self.shared.cancel.cancel();
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
        let mut admission = self.handle.shared.admission();
        if startup.swap(true, Ordering::AcqRel) {
            return false;
        }
        admission.pending_startups -= 1;
        self.handle.shared.publish_ready(&admission);
        true
    }
    /// Stop accepting/claiming new work, then drain admitted work.
    pub async fn draining(&self) {
        self.handle.draining().await;
    }

    /// Cooperative interruption after the drain allowance is exhausted.
    pub async fn cancelled(&self) {
        self.handle.shared.cancel.cancelled().await;
    }

    /// Whether drain has already been requested.
    pub fn is_draining(&self) -> bool {
        self.handle.is_draining()
    }

    /// Whether forced cooperative cancellation has already been requested.
    pub fn is_cancelled(&self) -> bool {
        self.handle.shared.cancel.is_cancelled()
    }
}

/// Phase budgets; these rely on tasks yielding to the runtime.
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

/// Event that initiated process shutdown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShutdownCause {
    /// External future or ShutdownHandle requested it.
    Requested,
    /// A registered critical component exited.
    ComponentExit(&'static str),
    /// No process work was registered; treated as a configuration failure.
    EmptySupervisor,
}

/// Complete process report, including teardown failures and unreaped work.
#[derive(Debug)]
pub struct ShutdownReport {
    /// Initial trigger, not a substitute for inspecting task/cleanup outcomes.
    pub cause: ShutdownCause,
    /// Directly joined tasks, in observation order.
    pub tasks: Vec<TaskRecord>,
    /// Successful finite tasks are counted instead of retained individually.
    pub completed_process_tasks: u64,
    /// Whether directly registered tasks remained after the drain phase.
    pub forced_cancellation: bool,
    /// Tasks for which abort was requested, sorted for stable reporting.
    pub abort_requested: Vec<&'static str>,
    /// Direct tasks whose completion could not be observed, sorted by name.
    pub unjoined: Vec<&'static str>,
    /// Finalizer outcomes, including explicitly skipped hooks.
    pub cleanup: CleanupReport,
}

impl ShutdownReport {
    /// Success excludes early exits, returned errors, panics, aborts,
    /// unobserved termination, and incomplete cleanup.
    pub fn is_success(&self) -> bool {
        self.cause != ShutdownCause::EmptySupervisor
            && self
                .tasks
                .iter()
                .all(|task| task.outcome == TaskOutcome::Stopped)
            && self.abort_requested.is_empty()
            && self.unjoined.is_empty()
            && self.cleanup.is_success()
    }

    /// Only direct tasks. This does NOT prove detached descendants terminated.
    pub fn all_direct_tasks_joined(&self) -> bool {
        self.unjoined.is_empty()
    }
}

impl std::fmt::Display for ShutdownReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let failures = self
            .tasks
            .iter()
            .filter(|t| t.outcome != TaskOutcome::Stopped)
            .count();
        write!(
            f,
            "shutdown {:?}: {failures} task failure(s), {} unjoined; {}",
            self.cause,
            self.unjoined.len(),
            self.cleanup
        )
    }
}
impl std::error::Error for ShutdownReport {}

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
pub struct Supervisor {
    components: Vec<Component>,
    cleanup: CleanupStack,
    handle: ShutdownHandle,
    budget: ShutdownBudget,
    process: Option<ProcessHandle>,
    queued: Option<mpsc::Receiver<process::QueuedProcess>>,
}

impl Supervisor {
    /// Build an initially unready supervisor, without spawning anything.
    pub fn new(budget: ShutdownBudget) -> Self {
        let handle = ShutdownHandle::new();
        handle.shared.admission().supervised = true;
        Self {
            components: Vec::new(),
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
        validation::name(name)?;
        if self
            .components
            .iter()
            .any(|component| component.name == name)
        {
            return Err(RegistrationError::Duplicate(name));
        }
        self.components.push(Component {
            name,
            factory: Box::new(move |signal| Box::pin(factory(signal)) as ComponentFuture),
        });
        self.handle.shared.admission().pending_startups += 1;
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

    /// Extract pending finalizers if application startup fails before running.
    /// The application must explicitly close this stack and retain both errors.
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
    pub fn run_until<F>(self, shutdown: F) -> impl Future<Output = ShutdownReport>
    where
        F: Future<Output = ()>,
    {
        let emergency = EmergencyShutdown(self.handle.clone());
        async move {
            let _emergency = emergency;
            scoped_dispatch::scope(self.drive_until(shutdown)).await
        }
    }

    async fn drive_until<F>(mut self, shutdown: F) -> ShutdownReport
    where
        F: Future<Output = ()>,
    {
        {
            let mut admission = self.handle.shared.admission();
            admission.running = true;
            self.handle.shared.publish_ready(&admission);
        }
        let mut tasks = TaskSet::default();
        for component in self.components.drain(..) {
            tasks.spawn_component(component, &self.handle);
        }
        tokio::pin!(shutdown);
        let cause = self.wait_for_shutdown(shutdown.as_mut(), &mut tasks).await;
        self.handle.request();
        tracing::info!(target: "batter", "shutdown drain started");
        tasks
            .collect_until(
                &mut self.queued,
                &self.handle,
                Instant::now() + self.budget.drain,
            )
            .await;
        let forced_cancellation = tasks.unfinished(&self.handle);
        self.handle.force_cancel();
        tasks
            .collect_until(
                &mut self.queued,
                &self.handle,
                Instant::now() + self.budget.cancel,
            )
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
        if !tasks.set.is_empty() {
            tasks
                .collect_until(
                    &mut self.queued,
                    &self.handle,
                    Instant::now() + self.budget.abort_reap,
                )
                .await;
        }
        let unjoined = tasks.sorted_names();
        // Joining a wrapper does not prove that its hidden children have ended.
        // Be conservative after panic or forced abort, particularly for servers.
        let unsafe_exit = !abort_requested.is_empty()
            || !unjoined.is_empty()
            || tasks
                .records
                .iter()
                .any(|r| matches!(r.outcome, TaskOutcome::Panicked | TaskOutcome::Aborted));
        let records = std::mem::take(&mut tasks.records);
        let completed_process_tasks = tasks.completed;
        drop(tasks);
        let cleanup = if unsafe_exit {
            self.cleanup.skip(SkipReason::UnsafeTaskExit)
        } else {
            self.cleanup.close(self.budget.cleanup).await
        };
        self.handle.shared.state.store(STOPPED, Ordering::Release);
        self.handle.shared.changed.notify_waiters();
        ShutdownReport {
            cause,
            tasks: records,
            completed_process_tasks,
            forced_cancellation,
            abort_requested,
            unjoined,
            cleanup,
        }
    }

    async fn wait_for_shutdown<F>(&mut self, shutdown: F, tasks: &mut TaskSet) -> ShutdownCause
    where
        F: Future<Output = ()>,
    {
        tokio::pin!(shutdown);
        if tasks.set.is_empty() && self.process.is_none() {
            ShutdownCause::EmptySupervisor
        } else {
            loop {
                tokio::select! {
                    biased;
                    // Draining cannot be starved by a stream of finite success.
                    _ = self.handle.draining() => break ShutdownCause::Requested,
                    _ = &mut shutdown => break ShutdownCause::Requested,
                    Some(result) = tasks.set.join_next_with_id(), if !tasks.set.is_empty() => {
                        if let Some(name) = tasks.record(result) {
                            self.handle.shared.admission().failed = true;
                            break ShutdownCause::ComponentExit(name);
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
        self.0.request();
        self.0.force_cancel();
    }
}

struct TaskExit {
    result: Result<(), BoxError>,
    expected: bool,
}

struct TaskMetadata {
    name: &'static str,
    finite: bool,
    abort: AbortHandle,
}

type TaskResult = Result<(Id, TaskExit), JoinError>;

#[derive(Default)]
struct TaskSet {
    set: JoinSet<TaskExit>,
    names: HashMap<Id, TaskMetadata>,
    records: Vec<TaskRecord>,
    completed: u64,
}

impl TaskSet {
    fn spawn_component(&mut self, component: Component, handle: &ShutdownHandle) {
        let name = component.name;
        let mut signal = handle.signal();
        signal.startup = Some(Arc::new(AtomicBool::new(false)));
        let handle = handle.clone();
        let span = tracing::info_span!(target: "batter", "batter.task", task = name);
        let abort = self.set.spawn(scoped_dispatch::scope(
            async move {
                let result = (component.factory)(signal).await;
                // Capture the state at completion, never at delayed observation.
                let expected = handle.is_draining();
                TaskExit { result, expected }
            }
            .instrument(span),
        ));
        self.names.insert(
            abort.id(),
            TaskMetadata {
                name,
                finite: false,
                abort,
            },
        );
    }

    fn spawn_process(&mut self, task: process::QueuedProcess) {
        let name = task.name;
        // The future already carries the submitting operation's span and
        // subscriber, independent of the coordinator's tracing context.
        let abort = self.set.spawn(task.future);
        self.names.insert(
            abort.id(),
            TaskMetadata {
                name,
                finite: true,
                abort,
            },
        );
    }

    fn sorted_names(&self) -> Vec<&'static str> {
        let mut values: Vec<_> = self.names.values().map(|entry| entry.name).collect();
        values.sort_unstable();
        values
    }

    fn pending(&self, handle: &ShutdownHandle) -> bool {
        !self.set.is_empty() || handle.shared.admission().finite_active != 0
    }

    fn unfinished(&self, handle: &ShutdownHandle) -> bool {
        self.names.values().any(|task| !task.abort.is_finished())
            || handle.shared.admission().finite_active != 0
    }

    // Membership means a result remains unobserved, not necessarily that the
    // task is still running. Preserve uncertainty only for actual abort requests.
    fn abort_unfinished(&self) -> Vec<&'static str> {
        let mut requested = Vec::new();
        for task in self.names.values() {
            if !task.abort.is_finished() {
                task.abort.abort();
                requested.push(task.name);
            }
        }
        requested.sort_unstable();
        requested
    }

    // None means a successful finite completion or expected critical stop.
    // Actual failures are retained and initiate shutdown before drain.
    fn record(&mut self, result: TaskResult) -> Option<&'static str> {
        let (id, mut outcome, error) = classify_task_result(result);
        let TaskMetadata { name, finite, .. } = self
            .names
            .remove(&id)
            .expect("every owned task has metadata");
        if finite && outcome == TaskOutcome::Stopped {
            outcome = TaskOutcome::Completed;
            self.completed = self.completed.saturating_add(1);
            tracing::debug!(target: "batter", task = name, ?outcome, "process task exit observed");
            return None;
        }
        let record = TaskRecord {
            name,
            outcome,
            error,
        };
        record.log_observation();
        self.records.push(record);
        (outcome != TaskOutcome::Stopped).then_some(name)
    }

    fn record_during_shutdown(&mut self, result: TaskResult, handle: &ShutdownHandle) {
        if self.record(result).is_some() {
            handle.shared.admission().failed = true;
        }
    }

    fn collect_ready(&mut self, handle: &ShutdownHandle) {
        // This nonblocking API can observe completed tasks even when Tokio's
        // cooperative poll budget is exhausted. Only this coordinator adds to
        // the JoinSet, so harvesting the currently spawned set is bounded.
        while let Some(result) = self.set.try_join_next_with_id() {
            self.record_during_shutdown(result, handle);
        }
    }

    async fn collect_until(
        &mut self,
        queued: &mut Option<mpsc::Receiver<process::QueuedProcess>>,
        handle: &ShutdownHandle,
        deadline: Instant,
    ) {
        while self.pending(handle) {
            tokio::select! {
                biased;
                _ = tokio::time::sleep_until(deadline) => break,
                Some(result) = self.set.join_next_with_id(), if !self.set.is_empty() => {
                    self.record_during_shutdown(result, handle);
                }
                Some(task) = receive_process(queued) => self.spawn_process(task),
            }
        }
        // Expired allowances stop waiting, not observation of results that are
        // already available. Reconcile before escalation or final reporting.
        self.collect_ready(handle);
    }
}

fn classify_task_result(result: TaskResult) -> (Id, TaskOutcome, Option<BoxError>) {
    match result {
        Ok((
            id,
            TaskExit {
                result: Ok(()),
                expected,
            },
        )) => (
            id,
            if expected {
                TaskOutcome::Stopped
            } else {
                TaskOutcome::UnexpectedExit
            },
            None,
        ),
        Ok((
            id,
            TaskExit {
                result: Err(error), ..
            },
        )) => (id, TaskOutcome::Failed, Some(error)),
        Err(error) => {
            let outcome = if error.is_panic() {
                TaskOutcome::Panicked
            } else {
                TaskOutcome::Aborted
            };
            (error.id(), outcome, Some(Box::new(error) as BoxError))
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
