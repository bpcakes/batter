//! Native invocation owns the complete borrowing work/finalization sequence.
#![allow(clippy::unwrap_used, reason = "a panic fails the test")]
#![allow(
    clippy::missing_assert_message,
    reason = "test names identify the contract"
)]
use batter_core::{
    command::CommandCause,
    operation::{Interruption, OperationContext, PhasedFailure},
};
use batter_runledger::{
    JobFinalization, JobWork, PhasedJob, PhasedJobFailure, PhasedJobHandler,
    test_support::{HandlerExit, HandlerInvocation},
};
use runledger_core::{
    jobs::{JobCompletion, JobFailure, JobType},
    prelude::async_trait,
};
use serde_json::{Value, json};
use std::{
    future::{Future, pending},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Waker},
    time::Duration,
};

#[path = "phased_handler/destruction.rs"]
mod destruction;

#[derive(Clone, Copy)]
enum Mode {
    Success,
    WorkError,
    WorkPanic,
    WorkTimeout,
    FinalError,
    BothError,
    FinalTimeout,
}
type Failure = PhasedJobFailure<usize, &'static str>;
struct Probe {
    mode: Mode,
    text: String,
    destroyed: Arc<AtomicBool>,
    finalized: Arc<AtomicBool>,
    work_context: Arc<Mutex<Option<OperationContext>>>,
    failure: Arc<Mutex<Option<Failure>>>,
}
impl Probe {
    fn new(mode: Mode) -> Self {
        Self {
            mode,
            text: "borrowed".into(),
            destroyed: Arc::default(),
            finalized: Arc::default(),
            work_context: Arc::default(),
            failure: Arc::default(),
        }
    }
}
struct Mark(Arc<AtomicBool>);
impl Drop for Mark {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

#[async_trait]
impl PhasedJobHandler for Probe {
    type Output = usize;
    type Error = &'static str;
    fn job_type(&self) -> JobType<'static> {
        JobType::new("test.phased")
    }
    async fn work(&self, scope: JobWork<'_>, _: Value) -> Result<usize, &'static str> {
        *self.work_context.lock().unwrap() = Some(scope.context().clone());
        let _mark = Mark(self.destroyed.clone());
        assert_eq!(scope.execution().checkpoint::<i32>().unwrap(), Some(0));
        scope
            .execution()
            .save_checkpoint(&1)
            .await
            .map_err(|_| "work checkpoint")?;
        tokio::task::yield_now().await;
        match self.mode {
            Mode::WorkError | Mode::BothError => Err("work error")?,
            Mode::WorkPanic => panic!("work panic"),
            Mode::WorkTimeout => pending::<()>().await,
            _ => {}
        }
        Ok(self.text.len())
    }
    async fn finalize(
        &self,
        scope: JobFinalization<'_, usize, &'static str>,
    ) -> Result<JobCompletion, &'static str> {
        assert!(
            self.destroyed.load(Ordering::SeqCst),
            "work destroyed before finalization"
        );
        assert_eq!(
            self.work_context.lock().unwrap().as_ref().unwrap().check(),
            Err(Interruption::Cancelled)
        );
        assert!(scope.context().check().is_ok());
        self.finalized.store(true, Ordering::SeqCst);
        let checkpoint = match scope.work().classify() {
            Ok(size) => *size as i32,
            Err(_) => -1,
        };
        scope
            .execution()
            .save_checkpoint(&checkpoint)
            .await
            .map_err(|_| "final checkpoint")?;
        match self.mode {
            Mode::FinalError | Mode::BothError => Err("final error"),
            Mode::FinalTimeout => pending().await,
            _ => Ok(JobCompletion::success()),
        }
    }
    fn failed(&self, failure: Failure) -> JobFailure {
        *self.failure.lock().unwrap() = Some(failure);
        JobFailure::terminal(
            "test.phases_failed",
            "The complete phase evidence was retained.",
        )
    }
}

#[tokio::test(start_paused = true)]
async fn borrowed_services_finalize_success_error_panic_and_work_timeout() {
    for mode in [
        Mode::Success,
        Mode::WorkError,
        Mode::WorkPanic,
        Mode::WorkTimeout,
        Mode::FinalError,
        Mode::BothError,
    ] {
        let probe = Probe::new(mode);
        let finalized = probe.finalized.clone();
        let failure = probe.failure.clone();
        let handler = PhasedJob::new(probe, Duration::from_secs(1)).unwrap();
        let run = HandlerInvocation::new(Duration::from_secs(3))
            .unwrap()
            .with_checkpoint(json!(0))
            .execute(&handler, json!({}))
            .await;
        assert!(finalized.load(Ordering::SeqCst));
        assert!(run.exit_hook_panics.is_empty());
        if matches!(mode, Mode::Success) {
            assert!(matches!(run.exit, HandlerExit::Returned(Ok(_))));
            assert_eq!(run.persisted.checkpoint, Some(json!(8)));
            assert!(failure.lock().unwrap().is_none());
            continue;
        }
        assert!(
            matches!(run.exit, HandlerExit::Returned(Err(ref error)) if error.code == "test.phases_failed")
        );
        let Some(PhasedJobFailure::Phases(PhasedFailure::Report(report))) =
            failure.lock().unwrap().take()
        else {
            panic!("both phase outcomes retained")
        };
        match mode {
            Mode::WorkError | Mode::BothError => assert!(matches!(
                report.work().classify().unwrap_err().returned_result(),
                Err(CommandCause::Failed("work error"))
            )),
            Mode::WorkPanic => assert!(matches!(
                report.work().classify().unwrap_err().returned_result(),
                Err(CommandCause::Panicked(_))
            )),
            Mode::WorkTimeout => assert!(matches!(
                report.work().classify().unwrap_err().returned_result(),
                Err(CommandCause::Interrupted(Interruption::DeadlineExceeded))
            )),
            Mode::FinalError => assert_eq!(report.work().classify().unwrap(), &8),
            _ => unreachable!(),
        }
        if matches!(mode, Mode::FinalError | Mode::BothError) {
            assert!(matches!(
                report
                    .finalization()
                    .classify()
                    .unwrap_err()
                    .returned_result(),
                Err(CommandCause::Failed("final error"))
            ));
        } else {
            assert!(report.finalization().is_success());
        }
        assert_eq!(
            run.persisted.checkpoint,
            Some(json!(if matches!(mode, Mode::FinalError) {
                8
            } else {
                -1
            }))
        );
    }
}

#[tokio::test(start_paused = true)]
async fn native_deadline_still_ends_a_pending_finalizer() {
    let probe = Probe::new(Mode::FinalTimeout);
    let finalized = probe.finalized.clone();
    let handler = PhasedJob::new(probe, Duration::from_secs(1)).unwrap();
    let run = HandlerInvocation::new(Duration::from_secs(3))
        .unwrap()
        .with_checkpoint(json!(0))
        .execute(&handler, json!({}))
        .await;
    assert!(finalized.load(Ordering::SeqCst));
    assert!(matches!(run.exit, HandlerExit::TimedOut));
    assert!(run.exit_hook_panics.is_empty());
}

#[tokio::test(start_paused = true)]
async fn ending_native_invocation_destroys_work_and_cancels_its_context() {
    let probe = Probe::new(Mode::WorkTimeout);
    let (destroyed, finalized, work_context) = (
        probe.destroyed.clone(),
        probe.finalized.clone(),
        probe.work_context.clone(),
    );
    let handler = PhasedJob::new(probe, Duration::from_secs(1)).unwrap();
    let mut run = Box::pin(
        HandlerInvocation::new(Duration::from_secs(3))
            .unwrap()
            .with_checkpoint(json!(0))
            .execute(&handler, json!({})),
    );
    assert!(
        run.as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    drop(run);
    assert!(destroyed.load(Ordering::SeqCst));
    assert!(
        !finalized.load(Ordering::SeqCst),
        "no finalization after native authority ends"
    );
    assert_eq!(
        work_context.lock().unwrap().as_ref().unwrap().check(),
        Err(Interruption::Cancelled)
    );
}

#[tokio::test(start_paused = true)]
async fn impossible_reserve_refuses_both_callbacks() {
    let probe = Probe::new(Mode::Success);
    let (finalized, work_context, failure) = (
        probe.finalized.clone(),
        probe.work_context.clone(),
        probe.failure.clone(),
    );
    for reserve in [
        Duration::ZERO,
        Duration::from_secs(366 * 24 * 60 * 60),
        Duration::MAX,
    ] {
        let invalid = Probe::new(Mode::Success);
        let work_context = invalid.work_context.clone();
        let finalized = invalid.finalized.clone();
        assert!(PhasedJob::new(invalid, reserve).is_err());
        assert!(work_context.lock().unwrap().is_none());
        assert!(!finalized.load(Ordering::SeqCst));
    }
    let handler = PhasedJob::new(probe, Duration::from_secs(4)).unwrap();
    let run = HandlerInvocation::new(Duration::from_secs(3))
        .unwrap()
        .execute(&handler, json!({}))
        .await;
    assert!(matches!(run.exit, HandlerExit::Returned(Err(_))));
    assert!(matches!(
        *failure.lock().unwrap(),
        Some(PhasedJobFailure::Rejected(_))
    ));
    assert!(!finalized.load(Ordering::SeqCst));
    assert!(work_context.lock().unwrap().is_none());
}
