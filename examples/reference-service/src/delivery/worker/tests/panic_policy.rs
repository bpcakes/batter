use super::super::phased::{Action, Invocation};
use super::*;
use batter::operation::{
    OperationOwner, PhaseOutcome,
    low_level::{ScopedPhases, drive_phases},
};
use runledger_core::jobs::JobFailureKind;
use std::task::{Context, Poll};

#[derive(Clone, Copy)]
enum FailureMode {
    None,
    Error,
    Construct,
    Poll,
    Drop,
}
struct Callback<T> {
    result: Option<Result<T, JobFailure>>,
    mode: FailureMode,
}
impl<T> Callback<T> {
    fn new(value: T, mode: FailureMode) -> Self {
        if matches!(mode, FailureMode::Construct) {
            panic!("secret construction payload");
        }
        Self {
            result: Some(if matches!(mode, FailureMode::Error) {
                Err(JobFailure::retryable(
                    "test.retry",
                    "Retryable application failure.",
                ))
            } else {
                Ok(value)
            }),
            mode,
        }
    }
}
impl<T: Unpin> Future for Callback<T> {
    type Output = Result<T, JobFailure>;
    fn poll(mut self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Self::Output> {
        if matches!(self.mode, FailureMode::Poll) {
            panic!("secret polling payload");
        }
        Poll::Ready(self.result.take().unwrap())
    }
}
impl<T> Drop for Callback<T> {
    fn drop(&mut self) {
        if matches!(self.mode, FailureMode::Drop) {
            panic!("secret destruction payload");
        }
    }
}
struct Phases {
    work: FailureMode,
    finalization: FailureMode,
    finalized: bool,
}
impl ScopedPhases for Phases {
    type Work = Action;
    type WorkError = JobFailure;
    type Final = JobCompletion;
    type FinalError = JobFailure;
    fn work(
        &mut self,
        _: &OperationContext,
    ) -> impl Future<Output = Result<Action, JobFailure>> + Send {
        Callback::new(Action::Complete, self.work)
    }
    fn finalize(
        &mut self,
        _: &OperationContext,
        _: &PhaseOutcome<Action, JobFailure>,
    ) -> impl Future<Output = Result<JobCompletion, JobFailure>> + Send {
        self.finalized = true;
        Callback::new(JobCompletion::success(), self.finalization)
    }
}

#[tokio::test]
async fn production_classifier_preserves_nonretryable_panics_after_finalization()
-> Result<(), batter::BoxError> {
    let worker = offline_worker().await?;
    let (_, payload) = owned_payload();
    let invocation = Invocation::new(&worker, &payload);
    for mode in [FailureMode::Construct, FailureMode::Poll, FailureMode::Drop] {
        for (work, finalization) in [
            (mode, FailureMode::None),
            (mode, FailureMode::Error),
            (FailureMode::None, mode),
        ] {
            let owner = OperationOwner::new(Duration::from_secs(10))?;
            let mut callbacks = Phases {
                work,
                finalization,
                finalized: false,
            };
            let failure = drive_phases(
                owner.context().reserve_finalization(FINAL_STATE_RESERVE)?,
                &mut callbacks,
            )
            .await
            .err()
            .unwrap();
            assert!(callbacks.finalized, "panic cannot bypass finalization");
            let native = invocation.failed(PhasedJobFailure::Phases(failure));
            assert_eq!(native.kind, JobFailureKind::Panicked);
            assert!(!native.kind.is_retryable());
            assert_eq!(native.code, "job.handler_panic");
            assert!(!format!("{native:?}").contains("secret"));
        }
    }
    // Ordinary application failures keep their retry policy.
    let owner = OperationOwner::new(Duration::from_secs(10))?;
    let mut callbacks = Phases {
        work: FailureMode::Error,
        finalization: FailureMode::None,
        finalized: false,
    };
    let failure = drive_phases(
        owner.context().reserve_finalization(FINAL_STATE_RESERVE)?,
        &mut callbacks,
    )
    .await
    .err()
    .unwrap();
    let native = invocation.failed(PhasedJobFailure::Phases(failure));
    assert!(native.kind.is_retryable());
    assert_eq!(native.code, "test.retry");
    Ok(())
}
