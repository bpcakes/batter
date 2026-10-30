use super::*;
use batter::operation::OperationPhases;
use batter::runledger::native::core::jobs::JobFailureKind;
use batter::runledger::test_support::{HandlerExit, HandlerInvocation};
use std::sync::Mutex;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::time::{advance, timeout};

const INVOCATION_TIME: Duration = Duration::from_secs(10);

/// The handler the catalog registers, with what it records and each
/// invocation's end as its observer task sees it.
fn greet() -> (Greet, UnboundedReceiver<String>, UnboundedReceiver<()>) {
    let (executed, recorded) = unbounded_channel();
    let (ended, endings) = unbounded_channel();
    (Greet { executed, ended }, recorded, endings)
}

/// Drives `handler` once through the worker's own dispatch, without a worker.
async fn drive(budget: Duration, handler: impl JobExecutionHandler, payload: Value) -> HandlerExit {
    let run = HandlerInvocation::new(budget)
        .expect("budget")
        .execute(&handler.into_job_handler(), payload)
        .await;
    assert!(run.exit_hook_panics.is_empty());
    run.exit
}

fn failure(exit: HandlerExit) -> JobFailure {
    match exit {
        HandlerExit::Returned(Err(failure)) => failure,
        other => panic!("expected the handler's own failure: {other:?}"),
    }
}

/// Derives `Greet`'s phases from the invocation and runs one step with them,
/// so a test can place that step on the invocation's own clock.
struct Step<F>(Mutex<Option<F>>);

fn step<F, Fut>(body: F) -> Step<F>
where
    F: FnOnce(OperationPhases) -> Fut + Send + 'static,
    Fut: Future<Output = Result<JobCompletion, JobFailure>> + Send + 'static,
{
    Step(Mutex::new(Some(body)))
}

#[async_trait]
impl<F, Fut> JobExecutionHandler for Step<F>
where
    F: FnOnce(OperationPhases) -> Fut + Send + 'static,
    Fut: Future<Output = Result<JobCompletion, JobFailure>> + Send + 'static,
{
    fn job_type(&self) -> JobType<'static> {
        JOB
    }

    async fn execute(
        &self,
        execution: JobExecution<'_>,
        _payload: Value,
    ) -> Result<JobCompletion, JobFailure> {
        let phases = job_phases(execution, FINAL_STATE_RESERVE).expect("work time remains");
        let body = self.0.lock().expect("step").take().expect("one invocation");
        body(phases).await
    }
}

#[tokio::test(start_paused = true)]
async fn a_greeting_is_recorded_and_the_exit_cancels_the_derived_phases() {
    let (handler, mut recorded, mut endings) = greet();
    let exit = drive(INVOCATION_TIME, handler, json!({"name": "valid"})).await;
    assert!(matches!(exit, HandlerExit::Returned(Ok(_))), "{exit:?}");
    assert_eq!(recorded.recv().await.as_deref(), Some("valid"));
    // The handler's observer task learns of the exit through the work phase.
    let ended = timeout(INVOCATION_TIME, endings.recv()).await;
    assert!(matches!(ended, Ok(Some(()))), "{ended:?}");
}

#[tokio::test(start_paused = true)]
async fn application_failures_keep_their_meaning() {
    let (handler, mut recorded, _endings) = greet();
    let missing = failure(drive(INVOCATION_TIME, handler, json!({})).await);
    assert_eq!(missing.kind, JobFailureKind::Terminal);
    assert_eq!(missing.code, "facade.consumer.payload");
    assert!(recorded.recv().await.is_none());

    let (handler, recorded, _endings) = greet();
    drop(recorded);
    let closed = failure(drive(INVOCATION_TIME, handler, json!({"name": "valid"})).await);
    assert_eq!(closed.kind, JobFailureKind::Terminal);
    assert_eq!(closed.code, "facade.consumer.observer");
}

#[tokio::test(start_paused = true)]
async fn a_budget_inside_the_reserve_leaves_no_work_time() {
    let (handler, mut recorded, _endings) = greet();
    let refused = failure(drive(FINAL_STATE_RESERVE, handler, json!({"name": "valid"})).await);
    assert_eq!(refused.kind, JobFailureKind::Timeout);
    assert_eq!(refused.code, "facade.consumer.no_work_time");
    assert!(recorded.recv().await.is_none());
}

#[tokio::test(start_paused = true)]
async fn expired_work_is_interruption_while_finalization_time_remains() {
    let expired = step(|phases| async move {
        advance(INVOCATION_TIME - FINAL_STATE_RESERVE).await;
        assert!(phases.finalization().check().is_ok());
        greeting_name(phases.work(), json!({"name": "valid"}))
            .await
            .map(|_| JobCompletion::success())
    });
    let interrupted = failure(drive(INVOCATION_TIME, expired, json!({})).await);
    assert_eq!(interrupted.kind, JobFailureKind::Timeout);
    assert_eq!(interrupted.code, "facade.consumer.work_interrupted");
}

#[tokio::test(start_paused = true)]
async fn cancelled_work_and_recording_after_the_exit_are_interruptions() {
    let (export, mut exported) = unbounded_channel();
    let cancelled = step(move |phases| async move {
        phases.cancel_work();
        let interrupted = greeting_name(phases.work(), json!({"name": "valid"}))
            .await
            .expect_err("work was cancelled");
        assert!(phases.finalization().check().is_ok());
        // Recording that outlives the handler keeps the finalization phase.
        assert!(export.send(phases.finalization().clone()).is_ok());
        Err(interrupted)
    });
    let interrupted = failure(drive(INVOCATION_TIME, cancelled, json!({})).await);
    assert_eq!(interrupted.kind, JobFailureKind::Timeout);
    assert_eq!(interrupted.code, "facade.consumer.work_interrupted");

    // The invocation's exit ended the finalization phase too.
    let finalization = exported.recv().await.expect("exported");
    let (executed, mut received) = unbounded_channel();
    let late = record_greeting(&finalization, executed, "valid".into())
        .await
        .unwrap_err();
    assert_eq!(late.kind, JobFailureKind::Timeout);
    assert_eq!(late.code, "facade.consumer.record_interrupted");
    assert!(received.recv().await.is_none());
}
