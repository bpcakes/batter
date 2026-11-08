use super::*;
use batter::runledger::native::core::jobs::{JobFailureKind, JobHandler};
use batter::runledger::test_support::{HandlerExit, HandlerInvocation};
use std::sync::Mutex;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::time::{advance, timeout};

const INVOCATION_TIME: Duration = Duration::from_secs(10);

/// The handler the catalog registers, with what it records and each
/// invocation's end as its observer task sees it.
fn greet() -> (
    PhasedJob<Greet>,
    UnboundedReceiver<String>,
    UnboundedReceiver<()>,
) {
    let (executed, recorded) = unbounded_channel();
    let (ended, endings) = unbounded_channel();
    (
        PhasedJob::new(Greet { executed, ended }, FINAL_STATE_RESERVE).expect("positive reserve"),
        recorded,
        endings,
    )
}

/// Drives `handler` once through the worker's own dispatch, without a worker.
async fn drive(budget: Duration, handler: impl JobHandler, payload: Value) -> HandlerExit {
    let run = HandlerInvocation::new(budget)
        .expect("budget")
        .execute(&handler, payload)
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

#[tokio::test(start_paused = true)]
async fn a_greeting_is_recorded_and_native_exit_has_a_cancelled_work_scope() {
    let (handler, mut recorded, mut endings) = greet();
    let exit = drive(INVOCATION_TIME, handler, json!({"name": "valid"})).await;
    assert!(matches!(exit, HandlerExit::Returned(Ok(_))), "{exit:?}");
    assert_eq!(recorded.recv().await.as_deref(), Some("valid"));
    // The observer waits for native exit and checks work-scope cancellation.
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

enum Mode {
    ExpireWork,
    FinishedWork,
}
struct Probe {
    mode: Mode,
    work: Mutex<Option<OperationContext>>,
    finalization: UnboundedSender<OperationContext>,
}
#[async_trait]
impl PhasedJobHandler for Probe {
    type Output = String;
    type Error = JobFailure;
    fn job_type(&self) -> JobType<'static> {
        JOB
    }
    async fn work(&self, scope: JobWork<'_>, _: Value) -> Result<String, JobFailure> {
        *self.work.lock().unwrap() = Some(scope.context().clone());
        if matches!(self.mode, Mode::ExpireWork) {
            advance(INVOCATION_TIME - FINAL_STATE_RESERVE).await;
        }
        greeting_name(scope.context(), json!({"name":"valid"})).await
    }
    async fn finalize(
        &self,
        scope: JobFinalization<'_, String, JobFailure>,
    ) -> Result<JobCompletion, JobFailure> {
        assert!(scope.context().check().is_ok());
        let _ = self.finalization.send(scope.context().clone());
        match self.mode {
            Mode::ExpireWork => {
                assert_eq!(scope.context().remaining(), FINAL_STATE_RESERVE);
                assert!(!scope.work().is_success());
            }
            Mode::FinishedWork => {
                let work = self.work.lock().unwrap().clone().unwrap();
                let error = greeting_name(&work, json!({"name":"valid"}))
                    .await
                    .expect_err("finished work is cancelled");
                return Err(error);
            }
        }
        Ok(JobCompletion::success())
    }
    fn failed(&self, failure: PhasedJobFailure<String, JobFailure>) -> JobFailure {
        let (executed, _) = unbounded_channel();
        let (ended, _) = unbounded_channel();
        Greet { executed, ended }.failed(failure)
    }
}
fn probe(mode: Mode) -> (PhasedJob<Probe>, UnboundedReceiver<OperationContext>) {
    let (finalization, phases) = unbounded_channel();
    (
        PhasedJob::new(
            Probe {
                mode,
                work: Mutex::new(None),
                finalization,
            },
            FINAL_STATE_RESERVE,
        )
        .unwrap(),
        phases,
    )
}

#[tokio::test(start_paused = true)]
async fn expired_work_is_interruption_while_finalization_time_remains() {
    let (handler, _) = probe(Mode::ExpireWork);
    let interrupted = failure(drive(INVOCATION_TIME, handler, json!({})).await);
    assert_eq!(interrupted.kind, JobFailureKind::Timeout);
    assert_eq!(interrupted.code, "facade.consumer.work_interrupted");
}

#[tokio::test(start_paused = true)]
async fn finished_work_and_recording_after_the_exit_are_interruptions() {
    let (handler, mut phases) = probe(Mode::FinishedWork);
    let interrupted = failure(drive(INVOCATION_TIME, handler, json!({})).await);
    assert_eq!(interrupted.kind, JobFailureKind::Timeout);
    assert_eq!(interrupted.code, "facade.consumer.work_interrupted");
    let finalization = phases.recv().await.expect("exported");
    let (executed, mut received) = unbounded_channel();
    let late = record_greeting(&finalization, executed, "valid".into())
        .await
        .unwrap_err();
    assert_eq!(late.kind, JobFailureKind::Timeout);
    assert_eq!(late.code, "facade.consumer.record_interrupted");
    assert!(received.recv().await.is_none());
}
