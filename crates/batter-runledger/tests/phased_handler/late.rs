use super::*;
use batter_runledger::{LateJobFinalization, LateJobSettlement};
use runledger_core::jobs::{
    JobContext, JobExecution, JobExecutionError, JobExecutionServices, JobExecutionUpdate,
    JobHandler, JobInvocation, JobInvocationOwner,
};
use std::{pin::Pin, task::Poll};
use tokio::time::Instant;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Work {
    OnTime,
    Late,
    Error,
    PollPanic,
    DropPanic,
    Missing,
    Cancelled,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Settlement {
    Accept,
    Decline,
    Error,
    PollPanic,
    DropPanic,
    Late,
    DropLate,
    DropEndsInvocation,
    EndInvocation,
    Pending,
}

struct LateProbe {
    work: Work,
    settlement: Settlement,
    selected: AtomicBool,
    ordinary: AtomicBool,
    failure: Mutex<Option<Failure>>,
    owner: Arc<Mutex<Option<JobInvocationOwner>>>,
    destroyed: Arc<AtomicBool>,
}

struct WorkFuture<'a> {
    probe: &'a LateProbe,
    deadline: Instant,
}
impl Future for WorkFuture<'_> {
    type Output = Result<usize, &'static str>;
    fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Self::Output> {
        if self.probe.work == Work::Missing {
            return Poll::Pending;
        }
        if self.probe.work != Work::OnTime {
            std::thread::sleep(
                self.deadline.saturating_duration_since(Instant::now()) + Duration::from_millis(2),
            );
        }
        match self.probe.work {
            Work::Cancelled => {
                self.probe.owner.lock().unwrap().take();
            }
            Work::PollPanic => panic!("work panic"),
            Work::Error => return Poll::Ready(Err("work error")),
            _ => {}
        }
        Poll::Ready(Ok(73))
    }
}
impl Drop for WorkFuture<'_> {
    fn drop(&mut self) {
        if self.probe.work == Work::DropPanic {
            panic!("work destruction panic");
        }
    }
}

struct SettlementFuture<'a> {
    probe: &'a LateProbe,
    scope: LateJobFinalization<'a, usize>,
}
impl Future for SettlementFuture<'_> {
    type Output = Result<JobCompletion, &'static str>;
    fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Self::Output> {
        assert_eq!(self.scope.value(), &73);
        match self.probe.settlement {
            Settlement::Error => return Poll::Ready(Err("settlement error")),
            Settlement::PollPanic => panic!("settlement panic"),
            Settlement::Pending => return Poll::Pending,
            Settlement::Late => std::thread::sleep(
                self.scope
                    .context()
                    .deadline()
                    .saturating_duration_since(Instant::now())
                    + Duration::from_millis(2),
            ),
            Settlement::EndInvocation => {
                self.probe.owner.lock().unwrap().take();
            }
            _ => {}
        }
        Poll::Ready(Ok(JobCompletion::success()))
    }
}
impl Drop for SettlementFuture<'_> {
    fn drop(&mut self) {
        self.probe.destroyed.store(true, Ordering::SeqCst);
        if self.probe.settlement == Settlement::DropLate {
            std::thread::sleep(
                self.scope
                    .context()
                    .deadline()
                    .saturating_duration_since(Instant::now())
                    + Duration::from_millis(2),
            );
        }
        if self.probe.settlement == Settlement::DropEndsInvocation {
            self.probe.owner.lock().unwrap().take();
        }
        if self.probe.settlement == Settlement::DropPanic {
            panic!("settlement destruction panic");
        }
    }
}

// Forward concrete futures so return and future destruction are independently tested.
impl PhasedJobHandler for LateProbe {
    type Output = usize;
    type Error = &'static str;
    fn job_type(&self) -> JobType<'static> {
        JobType::new("test.late")
    }
    fn work<'a, 's, 'f>(
        &'a self,
        scope: JobWork<'s>,
        _: Value,
    ) -> Pin<Box<dyn Future<Output = Result<usize, &'static str>> + Send + 'f>>
    where
        'a: 'f,
        's: 'f,
        Self: 'f,
    {
        Box::pin(WorkFuture {
            probe: self,
            deadline: scope.context().deadline(),
        })
    }
    fn finalize<'a, 's, 'f>(
        &'a self,
        _: JobFinalization<'s, usize, &'static str>,
    ) -> Pin<Box<dyn Future<Output = Result<JobCompletion, &'static str>> + Send + 'f>>
    where
        'a: 'f,
        's: 'f,
        Self: 'f,
    {
        self.ordinary.store(true, Ordering::SeqCst);
        Box::pin(async { Ok(JobCompletion::success()) })
    }
    fn settle_late<'a>(
        &'a self,
        scope: LateJobFinalization<'a, usize>,
    ) -> LateJobSettlement<'a, &'static str> {
        self.selected.store(true, Ordering::SeqCst);
        if self.settlement == Settlement::Decline {
            scope.decline()
        } else {
            scope.settle(move |scope| SettlementFuture { probe: self, scope })
        }
    }
    fn failed(&self, failure: Failure) -> JobFailure {
        *self.failure.lock().unwrap() = Some(failure);
        JobFailure::terminal("test.failed", "Phases failed.")
    }
}

struct Services {
    deadline: Instant,
    invocation: JobInvocation,
}
#[async_trait]
impl JobExecutionServices for Services {
    fn deadline(&self) -> std::time::Instant {
        self.deadline.into_std()
    }
    fn remaining_budget(&self) -> Duration {
        self.deadline.saturating_duration_since(Instant::now())
    }
    fn invocation(&self) -> Option<JobInvocation> {
        Some(self.invocation.clone())
    }
    async fn persist_progress(&self, _: JobExecutionUpdate<'_>) -> Result<(), JobExecutionError> {
        Ok(())
    }
}

fn fixture(work: Work, settlement: Settlement) -> (LateProbe, Services, JobContext) {
    let owner = JobInvocationOwner::new();
    let services = Services {
        deadline: Instant::now() + Duration::from_millis(200),
        invocation: owner.invocation(),
    };
    let probe = LateProbe {
        work,
        settlement,
        selected: AtomicBool::new(false),
        ordinary: AtomicBool::new(false),
        failure: Mutex::new(None),
        owner: Arc::new(Mutex::new(Some(owner))),
        destroyed: Arc::default(),
    };
    let context = serde_json::from_value(json!({
        "job_id": "00000000-0000-0000-0000-000000000001", "run_number": 1,
        "attempt": 1, "organization_id": null, "worker_id": "late-worker", "checkpoint": null,
    }))
    .unwrap();
    (probe, services, context)
}

#[tokio::test]
async fn only_explicit_settlement_of_a_clean_late_value_can_recover() {
    for work in [
        Work::OnTime,
        Work::Late,
        Work::Error,
        Work::PollPanic,
        Work::DropPanic,
        Work::Missing,
        Work::Cancelled,
    ] {
        let (probe, services, context) = fixture(work, Settlement::Accept);
        let handler = PhasedJob::new(Borrowed(&probe), Duration::from_millis(180)).unwrap();
        let result = handler
            .execute_with_services(JobExecution::new(&context, &services), Value::Null)
            .await;
        assert_eq!(
            result.is_ok(),
            matches!(work, Work::OnTime | Work::Late),
            "{work:?}"
        );
        assert_eq!(
            probe.selected.load(Ordering::SeqCst),
            work == Work::Late,
            "{work:?}"
        );
    }
}

#[tokio::test]
async fn failed_settlement_retains_both_phases_and_cannot_recover() {
    for settlement in [
        Settlement::Accept,
        Settlement::Decline,
        Settlement::Error,
        Settlement::PollPanic,
        Settlement::DropPanic,
        Settlement::Late,
        Settlement::DropLate,
        Settlement::DropEndsInvocation,
        Settlement::EndInvocation,
        Settlement::Pending,
    ] {
        let (probe, services, context) = fixture(Work::Late, settlement);
        // A borrowed handler lets the test inspect retained evidence without a public escape hatch.
        let selected = &probe.selected;
        let ordinary = &probe.ordinary;
        let handler = PhasedJob::new(Borrowed(&probe), Duration::from_millis(180)).unwrap();
        let result = handler
            .execute_with_services(JobExecution::new(&context, &services), Value::Null)
            .await;
        assert!(selected.load(Ordering::SeqCst), "{settlement:?}");
        assert_eq!(
            ordinary.load(Ordering::SeqCst),
            settlement == Settlement::Decline
        );
        assert_eq!(
            result.is_ok(),
            settlement == Settlement::Accept,
            "{settlement:?}"
        );
        if settlement != Settlement::Accept {
            let Some(PhasedJobFailure::Phases(PhasedFailure::Report(report))) =
                probe.failure.lock().unwrap().take()
            else {
                panic!("retained report")
            };
            let work = report.work().classify().unwrap_err();
            assert_eq!(work.returned_result().unwrap(), &73);
            assert_eq!(
                work.interruption_after_return(),
                Some(Interruption::DeadlineExceeded)
            );
            if settlement == Settlement::Error {
                assert!(matches!(
                    report
                        .finalization()
                        .classify()
                        .unwrap_err()
                        .returned_result(),
                    Err(CommandCause::Failed("settlement error"))
                ));
            }
            if settlement == Settlement::DropPanic {
                assert!(
                    report
                        .finalization()
                        .classify()
                        .unwrap_err()
                        .destruction_panic()
                        .is_some()
                );
            }
            if matches!(
                settlement,
                Settlement::DropLate | Settlement::DropEndsInvocation
            ) {
                assert!(
                    report.finalization().is_success(),
                    "native authority is checked again after destruction"
                );
            }
        }
    }
}

// Preserve the callback futures themselves across the borrowing test wrapper.
struct Borrowed<'a>(&'a LateProbe);
impl PhasedJobHandler for Borrowed<'_> {
    type Output = usize;
    type Error = &'static str;
    fn job_type(&self) -> JobType<'static> {
        self.0.job_type()
    }
    fn work<'a, 's, 'f>(
        &'a self,
        scope: JobWork<'s>,
        payload: Value,
    ) -> Pin<Box<dyn Future<Output = Result<usize, &'static str>> + Send + 'f>>
    where
        'a: 'f,
        's: 'f,
        Self: 'f,
    {
        self.0.work(scope, payload)
    }
    fn finalize<'a, 's, 'f>(
        &'a self,
        scope: JobFinalization<'s, usize, &'static str>,
    ) -> Pin<Box<dyn Future<Output = Result<JobCompletion, &'static str>> + Send + 'f>>
    where
        'a: 'f,
        's: 'f,
        Self: 'f,
    {
        self.0.finalize(scope)
    }
    fn settle_late<'a>(
        &'a self,
        scope: LateJobFinalization<'a, usize>,
    ) -> LateJobSettlement<'a, &'static str> {
        self.0.settle_late(scope)
    }
    fn failed(&self, failure: Failure) -> JobFailure {
        self.0.failed(failure)
    }
}

#[tokio::test]
async fn destroying_native_invocation_destroys_pending_settlement() {
    let (probe, services, context) = fixture(Work::Late, Settlement::Pending);
    let handler = PhasedJob::new(Borrowed(&probe), Duration::from_millis(180)).unwrap();
    let mut future = Box::pin(
        handler.execute_with_services(JobExecution::new(&context, &services), Value::Null),
    );
    assert!(
        future
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    assert!(probe.selected.load(Ordering::SeqCst));
    drop(future);
    drop(probe.owner.lock().unwrap().take());
    assert!(probe.destroyed.load(Ordering::SeqCst));
    assert!(
        probe.failure.lock().unwrap().is_none(),
        "no result after invocation destruction"
    );
}
