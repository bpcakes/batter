use super::super::super::phased::{Action, Invocation, Progress};
use super::*;
use std::{
    future::Future,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll},
};
use tokio::sync::oneshot;

type Timer = Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>;

// Poll the actual production work future. For the nested-boundary case, let
// its deadline pass after the phase driver checked it but before Bulkhead
// checks it, with the real fence transaction waiting for an effect-row lock.
struct BetweenPolls<'a> {
    work: Pin<Box<dyn Future<Output = Result<Action, JobFailure>> + Send + 'a>>,
    invocation: &'a Invocation<'a>,
    deadline: tokio::time::Instant,
    release: Option<oneshot::Sender<()>>,
    nested: bool,
    fence_waiting: Arc<AtomicBool>,
    nested_returned: Arc<AtomicBool>,
    timer: Timer,
}
impl Future for BetweenPolls<'_> {
    type Output = Result<Action, JobFailure>;
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if self.nested && self.fence_waiting.load(Ordering::SeqCst) {
            assert!(matches!(
                self.invocation.current_progress(),
                Progress::StateChange
            ));
            let after_deadline = Arc::new(AtomicBool::new(false));
            let signal = after_deadline.clone();
            let deadline = self.deadline + Duration::from_millis(10);
            *self.timer.lock().unwrap() = Some(tokio::spawn(async move {
                tokio::time::sleep_until(deadline).await;
                signal.store(true, Ordering::SeqCst);
            }));
            // Another runtime thread must drive the timer wheel while this
            // poll is deliberately paused between the two boundary checks.
            let limit = std::time::Instant::now() + PROBE_BUDGET;
            tokio::task::block_in_place(|| {
                while !after_deadline.load(Ordering::SeqCst) {
                    assert!(
                        std::time::Instant::now() < limit,
                        "timer driver stayed live"
                    );
                    std::thread::sleep(Duration::from_millis(1));
                }
            });
            let result = self.work.as_mut().poll(cx);
            assert!(
                matches!(result, Poll::Ready(Ok(Action::FenceInterrupted))),
                "nested timeout must retain fence ambiguity, never infer dispatch"
            );
            self.nested_returned.store(true, Ordering::SeqCst);
            return result;
        }
        self.work.as_mut().poll(cx)
    }
}
impl Drop for BetweenPolls<'_> {
    fn drop(&mut self) {
        if let Some(release) = self.release.take() {
            let _ = release.send(());
        }
    }
}
struct PendingFence<'a> {
    invocation: Invocation<'a>,
    release: Mutex<Option<oneshot::Sender<()>>>,
    nested: bool,
    fence_waiting: Arc<AtomicBool>,
    nested_returned: Arc<AtomicBool>,
    timer: Timer,
}
impl PhasedJobHandler for PendingFence<'_> {
    type Output = Action;
    type Error = JobFailure;
    fn job_type(&self) -> JobType<'static> {
        JobType::new(DELIVERY_JOB_TYPE)
    }
    fn work<'a, 's, 'f>(
        &'a self,
        scope: JobWork<'s>,
        payload: Value,
    ) -> Pin<Box<dyn Future<Output = Result<Action, JobFailure>> + Send + 'f>>
    where
        'a: 'f,
        's: 'f,
        Self: 'f,
    {
        let deadline = scope.context().deadline();
        Box::pin(BetweenPolls {
            work: self.invocation.work(scope, payload),
            invocation: &self.invocation,
            deadline,
            release: self.release.lock().unwrap().take(),
            nested: self.nested,
            fence_waiting: self.fence_waiting.clone(),
            nested_returned: self.nested_returned.clone(),
            timer: self.timer.clone(),
        })
    }
    fn finalize<'a, 's, 'f>(
        &'a self,
        scope: JobFinalization<'s, Action, JobFailure>,
    ) -> Pin<Box<dyn Future<Output = Result<JobCompletion, JobFailure>> + Send + 'f>>
    where
        'a: 'f,
        's: 'f,
        Self: 'f,
    {
        self.invocation.finalize(scope)
    }
    fn failed(&self, failure: PhasedJobFailure<Action, JobFailure>) -> JobFailure {
        self.invocation.failed(failure)
    }
}

pub(super) async fn interruption(worker: &DeliveryWorker) -> Result<(), BoxError> {
    for nested in [false, true] {
        let (context, payload) = claimed(&worker.pool).await?;
        let mut locked = worker.pool.begin().await?;
        sqlx::query(
            "SELECT delivery_id FROM reference_delivery_effects WHERE delivery_id = $1 FOR UPDATE",
        )
        .bind(payload.delivery_id)
        .execute(&mut *locked)
        .await?;
        let (release, released) = oneshot::channel();
        let fence_waiting = Arc::new(AtomicBool::new(false));
        let nested_returned = Arc::new(AtomicBool::new(false));
        let timer = Timer::default();
        let probe = PendingFence {
            invocation: Invocation::new(worker, &payload),
            release: Mutex::new(Some(release)),
            nested,
            fence_waiting: fence_waiting.clone(),
            nested_returned: nested_returned.clone(),
            timer: timer.clone(),
        };
        let handler = PhasedJob::new(probe, PROBE_BUDGET - Duration::from_secs(2))?;
        let (run, unlocked, observed) = tokio::join!(
            HandlerInvocation::for_context(PROBE_BUDGET, context)?.execute(&handler, Value::Null),
            async {
                released.await?;
                locked.rollback().await?;
                Ok::<_, BoxError>(())
            },
            async {
                tokio::time::timeout(Duration::from_secs(1), async {
                    loop {
                        let waiting: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM pg_stat_activity WHERE datname = current_database() AND wait_event_type = 'Lock' AND query LIKE 'SELECT delivery_id FROM reference_delivery_effects%FOR UPDATE')")
                            .fetch_one(&worker.pool).await?;
                        if waiting {
                            fence_waiting.store(true, Ordering::SeqCst);
                            tokio::task::yield_now().await;
                            return Ok::<_, sqlx::Error>(());
                        }
                        tokio::time::sleep(Duration::from_millis(2)).await;
                    }
                }).await??;
                Ok::<_, BoxError>(())
            },
        );
        unlocked?;
        let timer = timer.lock().unwrap().take();
        if let Some(timer) = timer {
            timer.await?;
        }
        assert!(
            observed.is_ok(),
            "fence row lock must become blocked before work expires"
        );
        assert!(
            fence_waiting.load(Ordering::SeqCst),
            "real fence SQL was pending"
        );
        assert_eq!(
            nested_returned.load(Ordering::SeqCst),
            nested,
            "requested deadline ordering was exercised"
        );
        assert_eq!(returned_failure(run).code, CODE_OPERATION_BUDGET_EXHAUSTED);
        assert_state(
            worker,
            &payload,
            "RETRYABLE_UNDISPATCHED",
            ProviderAcceptance::RuledOut,
        )
        .await?;
    }
    // Commit the real fenced write, then withhold the work result. This models
    // the durable side of a lost acknowledgement without claiming to emulate
    // PostgreSQL's wire-level COMMIT response timing.
    let (context, payload) = claimed(&worker.pool).await?;
    let handler = PhasedJob::new(
        CommittedFence(Invocation::new(worker, &payload)),
        PROBE_BUDGET - Duration::from_millis(500),
    )?;
    let run = HandlerInvocation::for_context(PROBE_BUDGET, context)?
        .execute(&handler, Value::Null)
        .await;
    assert_eq!(returned_failure(run).code, CODE_OPERATION_BUDGET_EXHAUSTED);
    assert_state(
        worker,
        &payload,
        "RECONCILE_NEEDED",
        ProviderAcceptance::Possible,
    )
    .await?;
    Ok(())
}

struct CommittedFence<'a>(Invocation<'a>);
#[async_trait]
impl PhasedJobHandler for CommittedFence<'_> {
    type Output = Action;
    type Error = JobFailure;
    fn job_type(&self) -> JobType<'static> {
        JobType::new(DELIVERY_JOB_TYPE)
    }
    async fn work(&self, scope: JobWork<'_>, _: Value) -> Result<Action, JobFailure> {
        self.0.progress(Progress::StateChange);
        assert!(
            self.0
                .worker
                .mark_dispatch_possible(
                    scope.execution(),
                    self.0.payload,
                    ProviderEffectState::AwaitingAttempt
                )
                .await
                .map_err(state_failure)?
        );
        std::future::pending().await
    }
    async fn finalize(
        &self,
        scope: JobFinalization<'_, Action, JobFailure>,
    ) -> Result<JobCompletion, JobFailure> {
        self.0.finalize(scope).await
    }
    fn failed(&self, failure: PhasedJobFailure<Action, JobFailure>) -> JobFailure {
        self.0.failed(failure)
    }
}

async fn assert_state(
    worker: &DeliveryWorker,
    payload: &DeliveryJobPayload,
    expected: &str,
    possible: ProviderAcceptance,
) -> Result<(), BoxError> {
    let (state, acceptance, code): (String, bool, Option<String>) = sqlx::query_as(
        "SELECT state, acceptance_possible, outcome_code FROM reference_delivery_effects WHERE delivery_id = $1")
        .bind(payload.delivery_id).fetch_one(&worker.pool).await?;
    assert_eq!(state, expected);
    assert_eq!(acceptance, possible == ProviderAcceptance::Possible);
    assert_eq!(code.as_deref(), Some(CODE_OPERATION_BUDGET_EXHAUSTED));
    Ok(())
}
