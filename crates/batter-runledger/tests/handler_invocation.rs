//! One handler invocation driven without a worker, queue or database. The
//! worker's own exit paths stay covered by Runledger's tests; these prove the
//! facility builds the execution the worker builds, classifies the exit by the
//! worker's acceptance rule, and leaves the test no invocation obligation.

use batter_core::operation::{Interruption, OperationContext, OperationError};
use batter_runledger::test_support::{HandlerExit, HandlerInvocation, PersistedProgress};
use batter_runledger::{JobPhasesRejection, job_phases};
use runledger_core::jobs::{
    JobCompletion, JobContext, JobContract, JobExecution, JobExecutionHandler, JobFailure,
    JobFailureKind, JobHandler, JobInvocation, JobSpec, JobType, TypedJobHandler,
};
use runledger_core::prelude::async_trait;
use serde_json::{Value, json};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::Poll;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::{Instant, advance, sleep_until};

const BUDGET: Duration = Duration::from_secs(10);
const RESERVE: Duration = Duration::from_millis(500);

fn rejection(rejection: JobPhasesRejection) -> JobFailure {
    match rejection {
        JobPhasesRejection::Exhausted | JobPhasesRejection::Ended => {
            JobFailure::timeout("probe.no_work_time", "No work time remained.")
        }
        JobPhasesRejection::Unsupported | JobPhasesRejection::Reserve(_) => {
            JobFailure::terminal("probe.unsupported", "Phases unavailable.")
        }
    }
}

/// What a probe reports to its test from inside the handler.
enum Report {
    /// The native deadline and the remaining budget, read together at the first poll.
    Deadline {
        deadline: std::time::Instant,
        remaining: Duration,
    },
    Work(OperationContext),
    /// Whether the invocation had ended when the parked handler future was destroyed.
    Destroyed {
        invocation_ended: bool,
    },
}

/// Held across a park, so it is dropped with the handler future.
struct DestructionProbe {
    invocation: JobInvocation,
    report: mpsc::UnboundedSender<Report>,
}

impl Drop for DestructionProbe {
    fn drop(&mut self) {
        let invocation_ended = self.invocation.has_ended();
        // A closed receiver fails the test's own read; a destructor must not panic.
        let _ = self.report.send(Report::Destroyed { invocation_ended });
    }
}

fn probe() -> (Probe, mpsc::UnboundedReceiver<Report>) {
    let (report, reports) = mpsc::unbounded_channel();
    (
        Probe {
            invoked: Arc::new(AtomicUsize::new(0)),
            report,
        },
        reports,
    )
}

/// One handler whose payload selects its behaviour after deriving phases.
struct Probe {
    invoked: Arc<AtomicUsize>,
    report: mpsc::UnboundedSender<Report>,
}

impl Probe {
    fn send(&self, report: Report) {
        self.report.send(report).expect("test receives reports");
    }
}

#[async_trait]
impl JobExecutionHandler for Probe {
    fn job_type(&self) -> JobType<'static> {
        JobType::new("probe.handler")
    }

    async fn execute(
        &self,
        execution: JobExecution<'_>,
        payload: Value,
    ) -> Result<JobCompletion, JobFailure> {
        self.send(Report::Deadline {
            deadline: execution.deadline(),
            remaining: execution.remaining_budget(),
        });
        let phases = job_phases(execution, RESERVE).map_err(rejection)?;
        self.send(Report::Work(phases.work().clone()));
        match payload["mode"].as_str().unwrap_or("success") {
            "park" => {
                let _destruction = DestructionProbe {
                    invocation: execution.invocation().expect("owned exit claim"),
                    report: self.report.clone(),
                };
                std::future::pending::<()>().await;
            }
            "panic" => panic!("probe panicked"),
            "hook_panic" => execution
                .invocation()
                .expect("owned exit claim")
                .on_end(|| panic!("probe hook panicked")),
            "at_deadline" => sleep_until(Instant::from_std(execution.deadline())).await,
            "panic_at_deadline" => {
                sleep_until(Instant::from_std(execution.deadline())).await;
                panic!("probe panicked at the deadline");
            }
            // Timers round up to a millisecond tick; the paused clock moves exactly.
            "before_deadline" => {
                advance(
                    Instant::from_std(execution.deadline())
                        - Instant::now()
                        - Duration::from_nanos(1),
                )
                .await;
            }
            _ => {}
        }
        let worked = phases
            .work()
            .run("probe.work", |_| async {
                self.invoked.fetch_add(1, Ordering::SeqCst);
                Ok::<_, std::io::Error>(())
            })
            .await;
        let outcome = match (worked, payload["outcome"].as_str()) {
            (Err(OperationError::Interrupted(_)), _) => {
                Err(JobFailure::timeout("probe.unknown", "Work interrupted."))
            }
            (Err(OperationError::Failed(_)), _) => {
                Err(JobFailure::terminal("probe.failed", "Work failed."))
            }
            (Ok(()), Some("retryable")) => Err(JobFailure::retryable("probe.retry", "Try again.")),
            (Ok(()), Some("terminal")) => Err(JobFailure::terminal("probe.stop", "Give up.")),
            (Ok(()), _) => Ok(JobCompletion::with_output(json!({ "done": true }))),
        };
        phases
            .finalization()
            .run("probe.record", |_| async {
                execution
                    .save_checkpoint(&json!({ "recorded": true }))
                    .await
            })
            .await
            .map_err(|_| JobFailure::retryable("probe.unrecorded", "Not recorded."))?;
        outcome
    }
}

fn observed_deadline(reports: &mut mpsc::UnboundedReceiver<Report>) -> (Instant, Duration) {
    match reports.try_recv() {
        Ok(Report::Deadline {
            deadline,
            remaining,
        }) => (Instant::from_std(deadline), remaining),
        _ => panic!("the handler reports its deadline first"),
    }
}

fn observed_work(reports: &mut mpsc::UnboundedReceiver<Report>) -> OperationContext {
    match reports.try_recv() {
        Ok(Report::Work(work)) => work,
        _ => panic!("the handler reports its work context second"),
    }
}

fn observed_destruction(reports: &mut mpsc::UnboundedReceiver<Report>) -> bool {
    match reports.try_recv() {
        Ok(Report::Destroyed { invocation_ended }) => invocation_ended,
        _ => panic!("a parked handler reports its destruction third"),
    }
}

#[tokio::test(start_paused = true)]
async fn success_runs_work_and_finalization_under_the_declared_budget() {
    let (probe, mut reports) = probe();
    let handler = probe.into_job_handler();
    let start = Instant::now();
    let run = HandlerInvocation::new(BUDGET)
        .expect("budget")
        .execute(&handler, json!({}))
        .await;
    let HandlerExit::Returned(Ok(completion)) = run.exit else {
        panic!("success: {:?}", run.exit);
    };
    assert_eq!(completion.output(), Some(&json!({ "done": true })));
    assert_eq!(
        observed_deadline(&mut reports),
        (start + BUDGET, BUDGET),
        "the native deadline is the budget after execution started, and the \
         remaining budget reads the same instant"
    );
    let work = observed_work(&mut reports);
    assert_eq!(work.deadline(), start + BUDGET - RESERVE);
    assert_eq!(
        work.check(),
        Err(Interruption::Cancelled),
        "the invocation ended after the handler returned"
    );
    assert_eq!(
        run.persisted,
        PersistedProgress {
            progress_done: None,
            progress_total: None,
            checkpoint: Some(json!({ "recorded": true })),
        }
    );
    assert!(run.exit_hook_panics.is_empty());
}

#[tokio::test(start_paused = true)]
async fn the_budget_starts_when_execution_is_first_polled() {
    let (probe, mut reports) = probe();
    let handler = probe.into_job_handler();
    // Neither declaring the invocation nor constructing its future starts the clock.
    let invocation = HandlerInvocation::new(BUDGET).expect("budget");
    advance(BUDGET / 4).await;
    let execution = invocation.execute(&handler, json!({ "mode": "park" }));
    advance(BUDGET / 4).await;
    let first_poll = Instant::now();
    let run = execution.await;
    assert_eq!(run.exit, HandlerExit::TimedOut);
    assert_eq!(
        Instant::now(),
        first_poll + BUDGET,
        "a parked handler times out one whole budget after the first poll"
    );
    assert_eq!(
        observed_deadline(&mut reports),
        (first_poll + BUDGET, BUDGET),
        "the handler starts with the whole budget"
    );
}

#[tokio::test(start_paused = true)]
async fn retryable_and_terminal_failures_keep_the_handler_classification() {
    let (probe, _reports) = probe();
    let handler = probe.into_job_handler();
    for (outcome, kind, code) in [
        ("retryable", JobFailureKind::Retryable, "probe.retry"),
        ("terminal", JobFailureKind::Terminal, "probe.stop"),
    ] {
        let run = HandlerInvocation::new(BUDGET)
            .expect("budget")
            .execute(&handler, json!({ "outcome": outcome }))
            .await;
        let HandlerExit::Returned(Err(failure)) = run.exit else {
            panic!("{outcome}: {:?}", run.exit);
        };
        assert_eq!(failure.kind, kind);
        assert_eq!(failure.code, code);
        assert!(run.exit_hook_panics.is_empty());
    }
}

#[tokio::test(start_paused = true)]
async fn a_budget_inside_the_reserve_rejects_phases_before_any_work() {
    let (probe, mut reports) = probe();
    let invoked = Arc::clone(&probe.invoked);
    let handler = probe.into_job_handler();
    for budget in [RESERVE, RESERVE / 2] {
        let run = HandlerInvocation::new(budget)
            .expect("budget")
            .execute(&handler, json!({}))
            .await;
        let HandlerExit::Returned(Err(failure)) = run.exit else {
            panic!("{budget:?}: {:?}", run.exit);
        };
        assert_eq!(failure.kind, JobFailureKind::Timeout);
        assert_eq!(failure.code, "probe.no_work_time");
        observed_deadline(&mut reports);
        assert!(
            reports.try_recv().is_err(),
            "no phases were derived for {budget:?}"
        );
        assert_eq!(invoked.load(Ordering::SeqCst), 0);
        assert_eq!(run.persisted.checkpoint, None, "finalization never ran");
    }
}

#[tokio::test(start_paused = true)]
async fn destroying_the_execution_before_work_ends_the_invocation() {
    let (probe, mut reports) = probe();
    let invoked = Arc::clone(&probe.invoked);
    let handler = probe.into_job_handler();
    // Never polled: no handler ran, and dropping it is not an error.
    drop(
        HandlerInvocation::new(BUDGET)
            .expect("budget")
            .execute(&handler, json!({})),
    );
    assert!(reports.try_recv().is_err());

    // Polled until the handler parks before its work, then destroyed, as a
    // worker task abort would be.
    let mut execution = Box::pin(
        HandlerInvocation::new(BUDGET)
            .expect("budget")
            .execute(&handler, json!({ "mode": "park" })),
    );
    let first = std::future::poll_fn(|cx| Poll::Ready(execution.as_mut().poll(cx))).await;
    assert!(first.is_pending());
    observed_deadline(&mut reports);
    let work = observed_work(&mut reports);
    let (done, mut cancelled) = mpsc::unbounded_channel();
    let observer = work.clone();
    tokio::spawn(async move {
        observer.cancelled().await;
        done.send(()).expect("test receives");
    });
    tokio::task::yield_now().await;
    assert!(work.check().is_ok(), "the invocation is still running");
    drop(execution);
    assert!(
        !observed_destruction(&mut reports),
        "the handler future is destroyed before the invocation ends"
    );
    assert_eq!(work.check(), Err(Interruption::Cancelled));
    cancelled
        .recv()
        .await
        .expect("the spawned observer saw the exit");
    assert_eq!(invoked.load(Ordering::SeqCst), 0, "work never started");
}

#[tokio::test(start_paused = true)]
async fn the_budget_elapsing_destroys_the_handler_and_times_the_invocation_out() {
    let (probe, mut reports) = probe();
    let invoked = Arc::clone(&probe.invoked);
    let handler = probe.into_job_handler();
    let start = Instant::now();
    let run = HandlerInvocation::new(BUDGET)
        .expect("budget")
        .execute(&handler, json!({ "mode": "park" }))
        .await;
    assert_eq!(run.exit, HandlerExit::TimedOut);
    assert_eq!(Instant::now(), start + BUDGET);
    observed_deadline(&mut reports);
    let work = observed_work(&mut reports);
    assert!(
        !observed_destruction(&mut reports),
        "the elapsed budget destroys the handler future before the invocation ends"
    );
    assert_eq!(work.check(), Err(Interruption::Cancelled));
    assert_eq!(invoked.load(Ordering::SeqCst), 0);
    assert_eq!(run.persisted.checkpoint, None);
    assert!(run.exit_hook_panics.is_empty());
}

#[tokio::test(start_paused = true)]
async fn results_and_panics_count_only_strictly_before_the_deadline() {
    let (probe, _reports) = probe();
    let handler = probe.into_job_handler();
    let late = HandlerInvocation::new(BUDGET)
        .expect("budget")
        .execute(&handler, json!({ "mode": "at_deadline" }))
        .await;
    assert_eq!(
        late.exit,
        HandlerExit::TimedOut,
        "a result at the deadline is late, whichever future the timeout polled first"
    );
    let late_panic = HandlerInvocation::new(BUDGET)
        .expect("budget")
        .execute(&handler, json!({ "mode": "panic_at_deadline" }))
        .await;
    assert_eq!(
        late_panic.exit,
        HandlerExit::TimedOut,
        "a panic observed at the deadline is late too, as the worker classifies it"
    );
    let accepted = HandlerInvocation::new(BUDGET)
        .expect("budget")
        .execute(&handler, json!({ "mode": "before_deadline" }))
        .await;
    assert!(
        matches!(accepted.exit, HandlerExit::Returned(Err(ref failure)) if failure.code == "probe.unknown"),
        "one nanosecond earlier the handler's own classification of its expired work is returned: {:?}",
        accepted.exit
    );
}

#[tokio::test(start_paused = true)]
async fn panics_are_contained_and_end_the_invocation() {
    let (probe, mut reports) = probe();
    let handler = probe.into_job_handler();
    let run = HandlerInvocation::new(BUDGET)
        .expect("budget")
        .execute(&handler, json!({ "mode": "panic" }))
        .await;
    assert_eq!(run.exit, HandlerExit::Panicked("probe panicked".to_owned()));
    observed_deadline(&mut reports);
    assert_eq!(
        observed_work(&mut reports).check(),
        Err(Interruption::Cancelled)
    );
    assert!(run.exit_hook_panics.is_empty());
}

#[tokio::test(start_paused = true)]
async fn exit_hook_panics_are_retained_beside_the_handler_result() {
    let (probe, _reports) = probe();
    let handler = probe.into_job_handler();
    let run = HandlerInvocation::new(BUDGET)
        .expect("budget")
        .execute(&handler, json!({ "mode": "hook_panic" }))
        .await;
    assert!(matches!(run.exit, HandlerExit::Returned(Ok(_))));
    assert_eq!(run.exit_hook_panics.len(), 1);
    assert_eq!(
        run.exit_hook_panics
            .payloads()
            .next()
            .and_then(|payload| payload.downcast_ref::<&'static str>()),
        Some(&"probe hook panicked")
    );
}

struct Contract;

impl JobContract for Contract {
    type Payload = i64;
    fn spec() -> JobSpec {
        JobSpec::new(JobType::new("probe.typed")).expect("spec")
    }
}

/// A typed handler that only accepts services, and a legacy handler that
/// ignores them: both run through the worker's dispatch.
struct Typed;

#[async_trait]
impl TypedJobHandler for Typed {
    type Contract = Contract;
    async fn execute(&self, _: JobContext, _: i64) -> Result<JobCompletion, JobFailure> {
        Err(JobFailure::terminal("probe.legacy", "Services required."))
    }
    async fn execute_with_services(
        &self,
        execution: JobExecution<'_>,
        payload: i64,
    ) -> Result<JobCompletion, JobFailure> {
        job_phases(execution, Duration::ZERO).map_err(rejection)?;
        Ok(JobCompletion::with_output(json!(payload)))
    }
}

struct Legacy;

#[async_trait]
impl JobHandler for Legacy {
    fn job_type(&self) -> JobType<'static> {
        JobType::new("probe.legacy")
    }
    async fn execute(&self, context: JobContext, _: Value) -> Result<JobCompletion, JobFailure> {
        Ok(JobCompletion::with_output(json!(context.job_id)))
    }
}

fn assert_send<F: Future + Send>(_: &F) {}

#[tokio::test(start_paused = true)]
async fn typed_and_legacy_handlers_run_through_the_worker_dispatch() {
    let typed = Typed.into_job_handler();
    let run = HandlerInvocation::new(BUDGET)
        .expect("budget")
        .execute(&typed, json!(7))
        .await;
    assert!(
        matches!(run.exit, HandlerExit::Returned(Ok(ref completion)) if completion.output() == Some(&json!(7))),
        "{:?}",
        run.exit
    );
    let malformed = HandlerInvocation::new(BUDGET)
        .expect("budget")
        .execute(&typed, json!("seven"))
        .await;
    assert!(
        matches!(malformed.exit, HandlerExit::Returned(Err(ref failure)) if failure.code == "job.invalid_payload"),
        "{:?}",
        malformed.exit
    );

    let invocation = HandlerInvocation::new(BUDGET).expect("budget");
    let job_id = json!(invocation.context().job_id);
    let execution = invocation.execute(&Legacy, json!({}));
    assert_send(&execution);
    let run = execution.await;
    assert!(
        matches!(run.exit, HandlerExit::Returned(Ok(ref completion)) if completion.output() == Some(&job_id)),
        "{:?}",
        run.exit
    );
    let erased: Arc<dyn JobHandler> = Arc::new(Legacy);
    let run = HandlerInvocation::new(BUDGET)
        .expect("budget")
        .execute(&*erased, json!({}))
        .await;
    assert!(matches!(run.exit, HandlerExit::Returned(Ok(_))));
}

/// A hand-written dispatch, which the expanded async-trait signature permits,
/// that panics before it returns a future.
struct PanicsWhileDispatching;

#[async_trait]
impl JobHandler for PanicsWhileDispatching {
    fn job_type(&self) -> JobType<'static> {
        JobType::new("probe.dispatch_panic")
    }
    async fn execute(&self, _: JobContext, _: Value) -> Result<JobCompletion, JobFailure> {
        Ok(JobCompletion::success())
    }
    fn execute_with_services<'life0, 'life1, 'async_trait>(
        &'life0 self,
        _: JobExecution<'life1>,
        _: Value,
    ) -> Pin<Box<dyn Future<Output = Result<JobCompletion, JobFailure>> + Send + 'async_trait>>
    where
        'life0: 'async_trait,
        'life1: 'async_trait,
        Self: 'async_trait,
    {
        panic!("probe panicked while dispatching")
    }
}

#[tokio::test(start_paused = true)]
async fn a_panic_while_dispatching_is_contained_as_the_worker_contains_it() {
    // The worker calls the dispatch inside its unwind boundary, so a panic
    // before the handler's future exists is still a contained handler panic.
    let run = HandlerInvocation::new(BUDGET)
        .expect("budget")
        .execute(&PanicsWhileDispatching, json!({}))
        .await;
    assert_eq!(
        run.exit,
        HandlerExit::Panicked("probe panicked while dispatching".to_owned())
    );
    assert!(run.exit_hook_panics.is_empty());
}
