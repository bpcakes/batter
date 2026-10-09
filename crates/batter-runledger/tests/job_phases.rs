//! Invocation-derived operation phases without PostgreSQL. The native worker's
//! own exit paths are covered by Runledger's worker tests; these tests drive the
//! same `JobInvocationOwner` the worker uses through custom services.

use batter_core::{
    ConfigurationError,
    operation::{Interruption, OperationContext, OperationError, OperationPhases},
};
use batter_runledger::{JobPhasesRejection, job_phases};
use runledger_core::jobs::{
    JobCompletion, JobContext, JobContract, JobExecution, JobExecutionError, JobExecutionHandler,
    JobExecutionServices, JobExecutionUpdate, JobFailure, JobHandler, JobInvocation,
    JobInvocationOwner, JobSpec, JobType, TypedJobHandler,
};
use runledger_core::prelude::async_trait;
use serde_json::{Value, json};
use std::future::Future;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use std::task::{Context, Wake, Waker};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::{Instant, advance};

const DEADLINE: Duration = Duration::from_secs(10);
const RESERVE: Duration = Duration::from_millis(500);

/// Custom services that opted in: they own this invocation's exit.
struct Services {
    deadline: std::time::Instant,
    observation: JobInvocation,
    owner: Mutex<Option<JobInvocationOwner>>,
}

impl Services {
    fn new() -> Self {
        let owner = JobInvocationOwner::new();
        Self {
            deadline: (Instant::now() + DEADLINE).into_std(),
            observation: owner.invocation(),
            owner: Mutex::new(Some(owner)),
        }
    }

    fn deadline(&self) -> Instant {
        Instant::from_std(self.deadline)
    }

    /// The runtime ends the invocation; nothing reachable from phases can.
    fn end(&self) {
        drop(self.owner.lock().expect("owner").take());
    }
}

#[async_trait]
impl JobExecutionServices for Services {
    fn deadline(&self) -> std::time::Instant {
        self.deadline
    }
    fn remaining_budget(&self) -> Duration {
        self.deadline
            .saturating_duration_since(Instant::now().into_std())
    }
    async fn persist_progress(&self, _: JobExecutionUpdate<'_>) -> Result<(), JobExecutionError> {
        Ok(())
    }
    fn invocation(&self) -> Option<JobInvocation> {
        Some(self.observation.clone())
    }
}

/// Custom services written before the exit signal existed: the trait default.
struct LegacyServices(std::time::Instant);

#[async_trait]
impl JobExecutionServices for LegacyServices {
    fn deadline(&self) -> std::time::Instant {
        self.0
    }
    fn remaining_budget(&self) -> Duration {
        self.0.saturating_duration_since(Instant::now().into_std())
    }
    async fn persist_progress(&self, _: JobExecutionUpdate<'_>) -> Result<(), JobExecutionError> {
        Ok(())
    }
}

fn context() -> JobContext {
    serde_json::from_value(json!({
        "job_id": "00000000-0000-0000-0000-000000000001",
        "run_number": 1,
        "attempt": 1,
        "organization_id": null,
        "worker_id": "phases-worker",
        "checkpoint": null,
    }))
    .expect("job context")
}

/// A handler body: derive, then run one counted work factory.
async fn attempt(
    execution: JobExecution<'_>,
    reserve: Duration,
    invoked: &AtomicUsize,
) -> Result<(), JobPhasesRejection> {
    let phases = job_phases(execution, reserve)?;
    let _ = phases
        .work()
        .run("phases.work", |_| async {
            invoked.fetch_add(1, Ordering::SeqCst);
            Ok::<_, std::io::Error>(())
        })
        .await;
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn deadlines_come_from_the_absolute_invocation_deadline_with_checked_reserves() {
    let services = Services::new();
    let context = context();
    let execution = JobExecution::new(&context, &services);
    let deadline = services.deadline();
    // Delayed derivation does not move either deadline.
    advance(Duration::from_secs(3)).await;
    let phases = job_phases(execution, RESERVE).expect("positive reserve");
    assert_eq!(phases.work().deadline(), deadline - RESERVE);
    assert_eq!(phases.finalization().deadline(), deadline);
    let phases = job_phases(execution, Duration::ZERO).expect("no reserve");
    assert_eq!(phases.work().deadline(), deadline);
    assert_eq!(phases.finalization().deadline(), deadline);

    for (reserve, expected) in [
        (Duration::from_secs(366 * 24 * 60 * 60), "too large"),
        (Duration::MAX, "too large"),
    ] {
        assert!(
            matches!(
                job_phases(execution, reserve),
                Err(JobPhasesRejection::Reserve(ConfigurationError::TooLarge(_)))
            ),
            "{expected}"
        );
    }
    // Representable, but the subtraction leaves no future work time.
    assert_eq!(
        job_phases(execution, Duration::from_secs(364 * 24 * 60 * 60)).err(),
        Some(JobPhasesRejection::Exhausted)
    );

    // Equality at the cutoff rejects; one nanosecond earlier admits one nanosecond.
    advance(deadline - RESERVE - Instant::now() - Duration::from_nanos(1)).await;
    let last = job_phases(execution, RESERVE).expect("one nanosecond of work");
    assert_eq!(last.work().remaining(), Duration::from_nanos(1));
    advance(Duration::from_nanos(1)).await;
    assert_eq!(
        job_phases(execution, RESERVE).err(),
        Some(JobPhasesRejection::Exhausted)
    );
    assert!(job_phases(execution, Duration::ZERO).is_ok());
    advance(RESERVE).await;
    assert_eq!(
        job_phases(execution, Duration::ZERO).err(),
        Some(JobPhasesRejection::Exhausted),
        "an elapsed invocation has no work time even without a reserve"
    );
}

#[tokio::test(start_paused = true)]
async fn rejected_expired_or_abandoned_work_invokes_no_factory() {
    let invoked = AtomicUsize::new(0);
    let context = context();
    let legacy = LegacyServices((Instant::now() + DEADLINE).into_std());
    assert_eq!(
        attempt(JobExecution::new(&context, &legacy), RESERVE, &invoked).await,
        Err(JobPhasesRejection::Unsupported),
        "services without an exit claim are refused, not treated as never ending"
    );
    let services = Services::new();
    let execution = JobExecution::new(&context, &services);
    // An unpolled factory is inert.
    let phases = job_phases(execution, RESERVE).expect("phases");
    drop(phases.work().run("phases.unpolled", |_| async {
        invoked.fetch_add(1, Ordering::SeqCst);
        Ok::<_, std::io::Error>(())
    }));
    // Work expires after derivation; its preflight rejects, finalization runs.
    advance(DEADLINE - RESERVE).await;
    let expired = phases
        .work()
        .run("phases.expired", |_| async {
            invoked.fetch_add(1, Ordering::SeqCst);
            Ok::<_, std::io::Error>(())
        })
        .await;
    assert!(matches!(
        expired,
        Err(OperationError::Interrupted(Interruption::DeadlineExceeded))
    ));
    assert_eq!(invoked.load(Ordering::SeqCst), 0);
    assert_eq!(phases.finalization().remaining(), RESERVE);
    let recorded = phases
        .finalization()
        .run("phases.final_state", |_| async {
            Ok::<_, std::io::Error>("recorded")
        })
        .await;
    assert_eq!(recorded.expect("reserve remains"), "recorded");
    services.end();
    for derived in [phases.work(), phases.finalization()] {
        assert_eq!(derived.check(), Err(Interruption::Cancelled));
    }
    assert_eq!(
        attempt(execution, Duration::ZERO, &invoked).await,
        Err(JobPhasesRejection::Ended)
    );
    assert_eq!(invoked.load(Ordering::SeqCst), 0);
}

fn assert_cancelled(phases: &OperationPhases) {
    assert_eq!(phases.work().check(), Err(Interruption::Cancelled));
    assert_eq!(phases.finalization().check(), Err(Interruption::Cancelled));
}

#[tokio::test(start_paused = true)]
async fn phases_keep_operation_phase_semantics_for_both_reserve_choices() {
    for reserve in [Duration::ZERO, RESERVE] {
        let services = Services::new();
        let context = context();
        let execution = JobExecution::new(&context, &services);
        let phases = job_phases(execution, reserve).expect("phases");
        assert_eq!(phases.finalization().deadline(), services.deadline());
        phases.cancel_work();
        assert_eq!(phases.work().check(), Err(Interruption::Cancelled));
        assert!(phases.finalization().check().is_ok(), "{reserve:?}");
        assert!(!services.observation.has_ended());

        let exhausted = job_phases(execution, reserve).expect("phases");
        let child = exhausted
            .work()
            .child(DEADLINE)
            .expect("child")
            .into_context();
        advance(DEADLINE - reserve).await;
        assert_eq!(
            exhausted.work().check(),
            Err(Interruption::DeadlineExceeded)
        );
        if reserve.is_zero() {
            assert_eq!(
                exhausted.finalization().check(),
                Err(Interruption::DeadlineExceeded),
                "no interval outlives work without a reserve"
            );
        } else {
            assert!(exhausted.finalization().check().is_ok());
        }
        services.end();
        assert_cancelled(&phases);
        assert_cancelled(&exhausted);
        assert_eq!(child.check(), Err(Interruption::Cancelled));
    }
}

/// Spawn one observer that acknowledges it is waiting for cancellation.
async fn observe(
    context: OperationContext,
    done: mpsc::UnboundedSender<&'static str>,
    name: &'static str,
) {
    let (acknowledged, ready) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let mut cancelled = std::pin::pin!(context.cancelled());
        let first =
            std::future::poll_fn(|cx| std::task::Poll::Ready(cancelled.as_mut().poll(cx))).await;
        assert!(
            first.is_pending(),
            "{name} starts before the invocation ends"
        );
        acknowledged.send(()).expect("acknowledgement");
        cancelled.await;
        done.send(name).expect("report");
    });
    ready.await.expect("observer waits");
}

#[tokio::test(start_paused = true)]
async fn native_exit_reaches_every_retained_derived_observer() {
    let services = Services::new();
    let context = context();
    let execution = JobExecution::new(&context, &services);
    let copied = execution;
    let phases = job_phases(execution, RESERVE).expect("phases");
    let again = job_phases(copied, Duration::ZERO).expect("phases from a copied handle");
    let (done, mut reports) = mpsc::unbounded_channel();
    let child = phases.work().child(DEADLINE).expect("child");
    let observed = [
        ("work", phases.work().clone()),
        ("finalization", phases.finalization().clone()),
        ("child", child.context().clone()),
        ("copied", again.finalization().clone()),
    ];
    for (name, observed) in observed {
        observe(observed, done.clone(), name).await;
    }
    // Dropping the phases, the child owner and the handles keeps the link.
    let late = phases.finalization().clone();
    drop((phases, child, again));
    advance(Duration::from_secs(1)).await;
    assert!(
        reports.try_recv().is_err(),
        "nothing ends before the invocation"
    );
    services.end();
    let mut names = Vec::new();
    for _ in 0..4 {
        names.push(reports.recv().await.expect("observer report"));
    }
    names.sort_unstable();
    assert_eq!(names, ["child", "copied", "finalization", "work"]);
    // Late observers and later children see the end at once.
    late.cancelled().await;
    let after = late.child(DEADLINE).expect("late child");
    assert_eq!(after.context().check(), Err(Interruption::Cancelled));
}

struct PanickingWaker;

struct PanicOnDrop;

impl Drop for PanicOnDrop {
    fn drop(&mut self) {
        panic!("native panic payload destruction");
    }
}

impl Wake for PanickingWaker {
    fn wake(self: Arc<Self>) {
        std::panic::panic_any(PanicOnDrop);
    }
}

#[tokio::test(start_paused = true)]
async fn panicking_native_waiter_cannot_suppress_derived_cancellation() {
    for explicit_end in [true, false] {
        let services = Services::new();
        let context = context();
        let phases = job_phases(JobExecution::new(&context, &services), RESERVE).expect("phases");
        let child = phases.work().child(DEADLINE).expect("child");
        let waker = Waker::from(Arc::new(PanickingWaker));
        let mut waiter = std::pin::pin!(services.observation.ended());
        assert!(
            waiter
                .as_mut()
                .poll(&mut Context::from_waker(&waker))
                .is_pending()
        );
        let (done, mut reports) = mpsc::unbounded_channel();
        for (name, observed) in [
            ("work", phases.work().clone()),
            ("finalization", phases.finalization().clone()),
            ("child", child.context().clone()),
        ] {
            observe(observed, done.clone(), name).await;
        }

        let owner = services
            .owner
            .lock()
            .expect("owner")
            .take()
            .expect("active");
        if explicit_end {
            assert_eq!(owner.end().len(), 1);
        } else {
            drop(owner);
        }
        assert_cancelled(&phases);
        assert_eq!(child.context().check(), Err(Interruption::Cancelled));
        let mut names = Vec::new();
        for _ in 0..3 {
            names.push(
                tokio::time::timeout(Duration::from_secs(1), reports.recv())
                    .await
                    .expect("observer was notified")
                    .expect("observer report"),
            );
        }
        names.sort_unstable();
        assert_eq!(names, ["child", "finalization", "work"]);
    }
}

#[tokio::test(start_paused = true)]
async fn cancellation_flows_downward_and_invocations_stay_isolated() {
    let (first, second) = (Services::new(), Services::new());
    let context = context();
    let a = job_phases(JobExecution::new(&context, &first), RESERVE).expect("first");
    let a_again = job_phases(JobExecution::new(&context, &first), RESERVE).expect("again");
    let b = job_phases(JobExecution::new(&context, &second), RESERVE).expect("second");
    let left = a.work().child(DEADLINE).expect("left");
    let right = a.work().child(DEADLINE).expect("right");
    left.cancel();
    assert_eq!(left.context().check(), Err(Interruption::Cancelled));
    assert!(right.context().check().is_ok(), "siblings are isolated");
    assert!(a.work().check().is_ok() && a.finalization().check().is_ok());
    a.cancel_work();
    assert!(
        a_again.work().check().is_ok(),
        "separate derivations are siblings"
    );
    assert!(
        !first.observation.has_ended(),
        "no derived context ends the invocation"
    );
    first.end();
    assert_cancelled(&a);
    assert_cancelled(&a_again);
    assert!(b.work().check().is_ok() && b.finalization().check().is_ok());
    second.end();
    assert_cancelled(&b);
}

struct Probe(mpsc::UnboundedSender<OperationPhases>);

#[async_trait]
impl JobExecutionHandler for Probe {
    fn job_type(&self) -> JobType<'static> {
        JobType::new("phases.probe")
    }
    async fn execute(
        &self,
        execution: JobExecution<'_>,
        _: Value,
    ) -> Result<JobCompletion, JobFailure> {
        let phases = job_phases(execution, RESERVE)
            .map_err(|_| JobFailure::terminal("phases.rejected", "No phases."))?;
        self.0.send(phases).expect("report phases");
        Ok(JobCompletion::success())
    }
}

struct Contract;

impl JobContract for Contract {
    type Payload = Value;
    fn spec() -> JobSpec {
        JobSpec::new(JobType::new("phases.typed")).expect("spec")
    }
}

struct TypedProbe(mpsc::UnboundedSender<OperationPhases>);

#[async_trait]
impl TypedJobHandler for TypedProbe {
    type Contract = Contract;
    async fn execute(&self, _: JobContext, _: Value) -> Result<JobCompletion, JobFailure> {
        Err(JobFailure::terminal("phases.legacy", "Services required."))
    }
    async fn execute_with_services(
        &self,
        execution: JobExecution<'_>,
        _: Value,
    ) -> Result<JobCompletion, JobFailure> {
        let phases = job_phases(execution, Duration::ZERO)
            .map_err(|_| JobFailure::terminal("phases.rejected", "No phases."))?;
        self.0.send(phases).expect("report phases");
        Ok(JobCompletion::success())
    }
}

#[tokio::test(start_paused = true)]
async fn adapted_and_typed_handlers_forward_the_invocation() {
    let (sender, mut received) = mpsc::unbounded_channel();
    let handlers: [Arc<dyn JobHandler>; 2] = [
        Arc::new(Probe(sender.clone()).into_job_handler()),
        Arc::new(TypedProbe(sender).into_job_handler()),
    ];
    let context = context();
    for handler in handlers {
        let services = Services::new();
        let execution = JobExecution::new(&context, &services);
        assert!(
            handler
                .execute_with_services(execution, json!({}))
                .await
                .is_ok()
        );
        let phases = received.try_recv().expect("handler derived phases");
        assert!(phases.work().check().is_ok());
        services.end();
        assert_cancelled(&phases);
        let legacy = LegacyServices(services.deadline);
        let refused = handler
            .execute_with_services(JobExecution::new(&context, &legacy), json!({}))
            .await
            .expect_err("legacy services are refused");
        assert_eq!(refused.code, "phases.rejected");
    }
}
