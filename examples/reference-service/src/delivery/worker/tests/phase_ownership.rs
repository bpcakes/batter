use super::*;
use batter::{
    command::CommandCause,
    operation::{Interruption, PhasedFailure},
};
use tokio::sync::mpsc;

type WaitingWork = tokio::task::JoinHandle<Result<(), BulkheadError<()>>>;

struct WaitingProbe {
    admission: Bulkhead,
    started: mpsc::UnboundedSender<(OperationContext, WaitingWork)>,
}
#[async_trait]
impl PhasedJobHandler for WaitingProbe {
    type Output = ();
    type Error = JobFailure;
    fn job_type(&self) -> JobType<'static> {
        JobType::new("probe.waiting")
    }
    async fn work(&self, scope: JobWork<'_>, _: Value) -> Result<(), JobFailure> {
        let work = scope.context().clone();
        let admission = self.admission.clone();
        let waiting_context = work.clone();
        let mut waiting = Box::pin(async move {
            admission
                .run(&waiting_context, Admission::Wait, "probe.admission", |_| {
                    std::future::pending::<Result<(), ()>>()
                })
                .await
        });
        let first =
            std::future::poll_fn(|cx| std::task::Poll::Ready(waiting.as_mut().poll(cx))).await;
        assert!(
            first.is_pending(),
            "provider admission waits for held capacity"
        );
        self.started.send((work, tokio::spawn(waiting))).unwrap();
        std::future::pending().await
    }
    async fn finalize(
        &self,
        _: JobFinalization<'_, (), JobFailure>,
    ) -> Result<JobCompletion, JobFailure> {
        Ok(JobCompletion::success())
    }
    fn failed(&self, _: PhasedJobFailure<(), JobFailure>) -> JobFailure {
        JobFailure::terminal("probe.failed", "Probe failed.")
    }
}

#[tokio::test]
async fn invocation_exit_reaches_provider_admission_and_calls() -> Result<(), batter::BoxError> {
    let worker = offline_worker().await?;
    let independent = batter::operation::OperationOwner::new(Duration::from_secs(60))?;
    let _held = worker
        .admission
        .hold(independent.context(), Admission::Wait)
        .await?;
    let (context, payload) = owned_payload();
    let (started, mut starts) = mpsc::unbounded_channel();
    let handler = PhasedJob::new(
        WaitingProbe {
            admission: worker.admission.clone(),
            started,
        },
        FINAL_STATE_RESERVE,
    )?;
    let mut run = Box::pin(
        HandlerInvocation::for_context(Duration::from_secs(60), context)?
            .execute(&handler, Value::Null),
    );
    let first = std::future::poll_fn(|cx| std::task::Poll::Ready(run.as_mut().poll(cx))).await;
    assert!(first.is_pending());
    let (work, waiting) = starts.recv().await.unwrap();
    drop(run); // Native authority ends while work is pending.
    assert!(matches!(
        waiting.await?,
        Err(BulkheadError::NotAdmitted(AdmissionError::Interrupted(
            Interruption::Cancelled
        )))
    ));
    let request = provider_request(&payload);
    let refused = worker
        .provider
        .prepare_dispatch(&request.idempotency_key(), &request)?
        .execute(&work)
        .await?;
    assert_eq!(
        refused,
        DispatchOutcome::KnownNotDispatched {
            retry_not_before: Duration::ZERO
        }
    );
    assert!(
        independent.context().check().is_ok(),
        "only linked work stops"
    );
    Ok(())
}

struct ExpiringProbe {
    worker: DeliveryWorker,
    payload: DeliveryJobPayload,
}
#[async_trait]
impl PhasedJobHandler for ExpiringProbe {
    type Output = ();
    type Error = JobFailure;
    fn job_type(&self) -> JobType<'static> {
        JobType::new("probe.expiring")
    }
    async fn work(&self, _: JobWork<'_>, _: Value) -> Result<(), JobFailure> {
        std::future::pending().await
    }
    async fn finalize(
        &self,
        scope: JobFinalization<'_, (), JobFailure>,
    ) -> Result<JobCompletion, JobFailure> {
        assert!(matches!(
            scope.work().classify().unwrap_err().returned_result(),
            Err(CommandCause::Interrupted(Interruption::DeadlineExceeded))
        ));
        assert!(scope.context().check().is_ok());
        assert_eq!(scope.context().remaining(), FINAL_STATE_RESERVE);
        self.worker
            .admission_failure(
                scope.execution(),
                &self.payload,
                ProviderEffectState::AwaitingAttempt,
                &AdmissionError::Interrupted(Interruption::DeadlineExceeded),
            )
            .await
    }
    fn failed(&self, failure: PhasedJobFailure<(), JobFailure>) -> JobFailure {
        if let PhasedJobFailure::Phases(PhasedFailure::Report(report)) = failure
            && let Err(finalization) = report.finalization().classify()
            && let Err(CommandCause::Failed(error)) = finalization.returned_result()
        {
            return error.clone();
        }
        panic!("offline final-state storage must fail")
    }
}

#[tokio::test(start_paused = true)]
async fn provider_work_expiry_leaves_the_final_state_reserve() -> Result<(), batter::BoxError> {
    let worker = offline_worker().await?;
    let (context, payload) = owned_payload();
    let handler = PhasedJob::new(ExpiringProbe { worker, payload }, FINAL_STATE_RESERVE)?;
    let run = HandlerInvocation::for_context(Duration::from_secs(10), context)?
        .execute(&handler, Value::Null)
        .await;
    assert_eq!(returned_failure(run).code, CODE_STATE_UNAVAILABLE);
    Ok(())
}
