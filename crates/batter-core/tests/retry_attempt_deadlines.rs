use batter_core::{
    ConfigurationError,
    operation::Interruption,
    retry::{
        self, ReplaySafety, RetryDecision, RetryError, RetryExecutionError, RetryOptions,
        RetryPolicy, StopReason,
    },
};
use std::{
    future::Future,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU32, Ordering},
    },
    task::{Context, Poll},
    time::Duration,
};
use tokio::time::Instant;

fn legacy_error_attempts(error: RetryError<&'static str>) -> u32 {
    match error {
        RetryError::Stopped { attempts, .. } | RetryError::Interrupted { attempts, .. } => attempts,
    }
}

fn legacy_stop_code(reason: StopReason) -> u8 {
    match reason {
        StopReason::NotRetryable => 0,
        StopReason::ReplayForbidden => 1,
        StopReason::AttemptsExhausted => 2,
        StopReason::InsufficientBudget => 3,
    }
}

fn legacy_interruption_code(reason: Interruption) -> u8 {
    match reason {
        Interruption::Cancelled => 0,
        Interruption::DeadlineExceeded => 1,
    }
}

fn policy(attempts: u32, backoff: Duration) -> RetryPolicy {
    RetryPolicy::new(attempts, backoff, backoff).unwrap()
}

fn options(maximum: Duration) -> RetryOptions {
    RetryOptions::new().with_attempt_maximum(maximum).unwrap()
}

#[test]
fn legacy_retry_enums_remain_exhaustively_matchable() {
    assert_eq!(
        legacy_error_attempts(RetryError::Interrupted {
            attempts: 2,
            reason: Interruption::Cancelled,
            last_error: Some("first"),
        }),
        2
    );
    assert_eq!(legacy_stop_code(StopReason::InsufficientBudget), 3);
    assert_eq!(legacy_interruption_code(Interruption::DeadlineExceeded), 1);
}

#[tokio::test(start_paused = true)]
async fn invalid_attempt_maximum_is_rejected_before_factory_work() {
    for (maximum, expected) in [
        (
            Duration::ZERO,
            ConfigurationError::Zero("retry attempt maximum"),
        ),
        (
            Duration::from_secs(366 * 24 * 60 * 60),
            ConfigurationError::TooLarge("retry attempt maximum"),
        ),
    ] {
        assert_eq!(
            RetryOptions::new()
                .with_attempt_maximum(maximum)
                .unwrap_err(),
            expected
        );
    }
}

#[tokio::test(start_paused = true)]
async fn first_attempt_cap_beats_ready_completion_and_leaves_total_time() {
    let context = batter_core::operation::OperationOwner::new(Duration::from_secs(10))
        .unwrap()
        .into_context();
    let started = Instant::now();
    let factories = Arc::new(AtomicU32::new(0));
    let classifiers = Arc::new(AtomicU32::new(0));
    let result: Result<(), _> = retry::execute_with_options(
        &context,
        "read.first-cap",
        ReplaySafety::Idempotent,
        &policy(3, Duration::from_millis(100)),
        options(Duration::from_secs(1)),
        {
            let factories = Arc::clone(&factories);
            move |attempt| {
                factories.fetch_add(1, Ordering::SeqCst);
                assert_eq!(attempt.number, 1);
                assert_eq!(attempt.context.deadline(), started + Duration::from_secs(1));
                async {
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    Ok::<(), &'static str>(())
                }
            }
        },
        {
            let classifiers = Arc::clone(&classifiers);
            move |_| {
                classifiers.fetch_add(1, Ordering::SeqCst);
                RetryDecision::Retry
            }
        },
    )
    .await;

    assert!(matches!(
        result,
        Err(RetryExecutionError::AttemptDeadlineExceeded {
            attempts: 1,
            last_error: None,
        })
    ));
    assert_eq!(Instant::now() - started, Duration::from_secs(1));
    assert!(context.check().is_ok());
    assert_eq!(factories.load(Ordering::SeqCst), 1);
    assert_eq!(classifiers.load(Ordering::SeqCst), 0);
}

#[tokio::test(start_paused = true)]
async fn later_attempt_cap_starts_after_backoff_and_retains_previous_error() {
    let context = batter_core::operation::OperationOwner::new(Duration::from_secs(10))
        .unwrap()
        .into_context();
    let started = Instant::now();
    let factories = Arc::new(AtomicU32::new(0));
    let classifiers = Arc::new(AtomicU32::new(0));
    let result: Result<(), _> = retry::execute_with_options(
        &context,
        "read.later-cap",
        ReplaySafety::Idempotent,
        &policy(3, Duration::from_millis(100)),
        options(Duration::from_millis(500)),
        {
            let factories = Arc::clone(&factories);
            move |attempt| {
                factories.fetch_add(1, Ordering::SeqCst);
                async move {
                    if attempt.number == 1 {
                        assert_eq!(
                            attempt.context.deadline(),
                            started + Duration::from_millis(500)
                        );
                        return Err("first");
                    }
                    assert_eq!(
                        attempt.context.deadline(),
                        started + Duration::from_millis(600)
                    );
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    Ok(())
                }
            }
        },
        {
            let classifiers = Arc::clone(&classifiers);
            move |_| {
                classifiers.fetch_add(1, Ordering::SeqCst);
                RetryDecision::Retry
            }
        },
    )
    .await;

    assert!(matches!(
        result,
        Err(RetryExecutionError::AttemptDeadlineExceeded {
            attempts: 2,
            last_error: Some("first"),
        })
    ));
    assert_eq!(Instant::now() - started, Duration::from_millis(600));
    assert_eq!(factories.load(Ordering::SeqCst), 2);
    assert_eq!(classifiers.load(Ordering::SeqCst), 1);
}

#[tokio::test(start_paused = true)]
async fn shorter_total_deadline_is_not_relabelled_as_attempt_expiration() {
    let context = batter_core::operation::OperationOwner::new(Duration::from_millis(400))
        .unwrap()
        .into_context();
    let total_deadline = context.deadline();
    let result: Result<(), _> = retry::execute_with_options(
        &context,
        "read.short-total",
        ReplaySafety::Idempotent,
        &policy(2, Duration::from_millis(10)),
        options(Duration::from_secs(1)),
        |attempt| async move {
            assert_eq!(attempt.context.deadline(), total_deadline);
            tokio::time::sleep(Duration::from_secs(1)).await;
            Ok::<(), &'static str>(())
        },
        |_| panic!("an interrupted attempt must not be classified"),
    )
    .await;

    assert!(matches!(
        result,
        Err(RetryExecutionError::Interrupted {
            attempts: 1,
            reason: Interruption::DeadlineExceeded,
            last_error: None,
        })
    ));
}

#[tokio::test(start_paused = true)]
async fn tied_attempt_and_total_deadlines_report_total_expiration() {
    let context = batter_core::operation::OperationOwner::new(Duration::from_millis(500))
        .unwrap()
        .into_context();
    let total_deadline = context.deadline();
    let result: Result<(), _> = retry::execute_with_options(
        &context,
        "read.tied-deadline",
        ReplaySafety::Idempotent,
        &policy(2, Duration::from_millis(10)),
        options(Duration::from_millis(500)),
        |attempt| async move {
            assert_eq!(attempt.context.deadline(), total_deadline);
            tokio::time::sleep(Duration::from_millis(500)).await;
            Ok::<(), &'static str>(())
        },
        |_| panic!("an interrupted attempt must not be classified"),
    )
    .await;

    assert!(matches!(
        result,
        Err(RetryExecutionError::Interrupted {
            attempts: 1,
            reason: Interruption::DeadlineExceeded,
            last_error: None,
        })
    ));
}

#[tokio::test(start_paused = true)]
async fn options_without_attempt_cap_preserve_total_expiration() {
    let context = batter_core::operation::OperationOwner::new(Duration::from_millis(500))
        .unwrap()
        .into_context();
    let result: Result<(), _> = retry::execute_with_options(
        &context,
        "read.default-options",
        ReplaySafety::Idempotent,
        &policy(2, Duration::from_millis(10)),
        RetryOptions::default(),
        |_| async {
            tokio::time::sleep(Duration::from_millis(500)).await;
            Ok::<(), &'static str>(())
        },
        |_| panic!("an interrupted attempt must not be classified"),
    )
    .await;

    assert!(matches!(
        result,
        Err(RetryExecutionError::Interrupted {
            attempts: 1,
            reason: Interruption::DeadlineExceeded,
            last_error: None,
        })
    ));
}

#[tokio::test(start_paused = true)]
async fn later_attempt_uses_only_total_time_remaining_after_backoff() {
    let context = batter_core::operation::OperationOwner::new(Duration::from_millis(500))
        .unwrap()
        .into_context();
    let total_deadline = context.deadline();
    let started = Instant::now();
    let result: Result<(), _> = retry::execute_with_options(
        &context,
        "read.remaining-total",
        ReplaySafety::Idempotent,
        &policy(3, Duration::from_millis(200)),
        options(Duration::from_millis(400)),
        |attempt| async move {
            if attempt.number == 1 {
                return Err("first");
            }
            assert_eq!(Instant::now() - started, Duration::from_millis(200));
            assert_eq!(attempt.context.deadline(), total_deadline);
            tokio::time::sleep(Duration::from_millis(300)).await;
            Ok(())
        },
        |_| RetryDecision::Retry,
    )
    .await;

    assert!(matches!(
        result,
        Err(RetryExecutionError::Interrupted {
            attempts: 2,
            reason: Interruption::DeadlineExceeded,
            last_error: Some("first"),
        })
    ));
    assert_eq!(Instant::now() - started, Duration::from_millis(500));
}

#[tokio::test(start_paused = true)]
async fn shortened_work_context_remains_the_total_retry_boundary() {
    let parent = batter_core::operation::OperationOwner::new(Duration::from_secs(1))
        .unwrap()
        .into_context();
    let phases = parent
        .reserve_finalization(Duration::from_millis(300))
        .unwrap();
    let work_deadline = phases.work().deadline();
    let result: Result<(), _> = retry::execute_with_options(
        phases.work(),
        "read.work-phase",
        ReplaySafety::Idempotent,
        &policy(2, Duration::from_millis(10)),
        options(Duration::from_secs(2)),
        |attempt| async move {
            assert_eq!(attempt.context.deadline(), work_deadline);
            tokio::time::sleep(Duration::from_secs(2)).await;
            Ok::<(), &'static str>(())
        },
        |_| panic!("an interrupted attempt must not be classified"),
    )
    .await;

    assert!(matches!(
        result,
        Err(RetryExecutionError::Interrupted {
            attempts: 1,
            reason: Interruption::DeadlineExceeded,
            last_error: None,
        })
    ));
    assert_eq!(parent.remaining(), Duration::from_millis(300));
    phases
        .finalization()
        .run("read.finalize", |_| async {
            tokio::time::sleep(Duration::from_millis(200)).await;
            Ok::<(), &'static str>(())
        })
        .await
        .unwrap();
}

#[tokio::test(start_paused = true)]
async fn tighter_attempt_cap_inside_work_preserves_finalization_reserve() {
    let parent = batter_core::operation::OperationOwner::new(Duration::from_secs(1))
        .unwrap()
        .into_context();
    let phases = parent
        .reserve_finalization(Duration::from_millis(300))
        .unwrap();
    let started = Instant::now();
    let result: Result<(), _> = retry::execute_with_options(
        phases.work(),
        "read.capped-work-phase",
        ReplaySafety::Idempotent,
        &policy(2, Duration::from_millis(10)),
        options(Duration::from_millis(200)),
        |attempt| async move {
            assert_eq!(
                attempt.context.deadline(),
                started + Duration::from_millis(200)
            );
            std::future::pending::<Result<(), &'static str>>().await
        },
        |_| panic!("an interrupted attempt must not be classified"),
    )
    .await;

    assert!(matches!(
        result,
        Err(RetryExecutionError::AttemptDeadlineExceeded {
            attempts: 1,
            last_error: None,
        })
    ));
    assert_eq!(parent.remaining(), Duration::from_millis(800));
    phases
        .finalization()
        .run("read.capped-finalize", |_| async {
            tokio::time::sleep(Duration::from_millis(250)).await;
            Ok::<(), &'static str>(())
        })
        .await
        .unwrap();
}

#[tokio::test(start_paused = true)]
async fn cancellation_wins_over_an_already_expired_total_deadline() {
    let owner = batter_core::operation::OperationOwner::at(
        batter_core::operation::RootDeadline::at(Instant::now()),
    );
    owner.cancel();
    let context = owner.context();
    let factories = Arc::new(AtomicU32::new(0));
    let result: Result<(), _> = retry::execute_with_options(
        context,
        "read.cancelled",
        ReplaySafety::Idempotent,
        &policy(2, Duration::from_millis(10)),
        options(Duration::from_secs(1)),
        {
            let factories = Arc::clone(&factories);
            move |_| {
                factories.fetch_add(1, Ordering::SeqCst);
                async { Ok::<(), &'static str>(()) }
            }
        },
        |_| RetryDecision::Retry,
    )
    .await;

    assert!(matches!(
        result,
        Err(RetryExecutionError::Interrupted {
            attempts: 0,
            reason: Interruption::Cancelled,
            last_error: None,
        })
    ));
    assert_eq!(factories.load(Ordering::SeqCst), 0);
}

#[tokio::test(start_paused = true)]
async fn cancellation_wins_at_the_attempt_deadline_and_stays_downward() {
    let owner = batter_core::operation::OperationOwner::new(Duration::from_secs(10)).unwrap();
    let context = owner.context();
    let retry_policy = policy(2, Duration::from_millis(10));
    let mut execution = Box::pin(retry::execute_with_options(
        context,
        "read.cancel-at-cap",
        ReplaySafety::Idempotent,
        &retry_policy,
        options(Duration::from_secs(1)),
        |_| std::future::pending::<Result<(), &'static str>>(),
        |_| panic!("an interrupted attempt must not be classified"),
    ));

    std::future::poll_fn(|task_context| match execution.as_mut().poll(task_context) {
        Poll::Pending => Poll::Ready(()),
        Poll::Ready(_) => panic!("attempt unexpectedly completed"),
    })
    .await;
    tokio::time::advance(Duration::from_secs(1)).await;
    owner.cancel();
    let result = execution.await;

    assert!(matches!(
        result,
        Err(RetryExecutionError::Interrupted {
            attempts: 1,
            reason: Interruption::Cancelled,
            last_error: None,
        })
    ));

    let local_parent = batter_core::operation::OperationOwner::new(Duration::from_secs(10))
        .unwrap()
        .into_context();
    let local_owner = Arc::new(local_parent.child(Duration::from_secs(10)).unwrap());
    let local_context = local_owner.context().clone();
    let result: Result<(), _> = retry::execute_with_options(
        &local_context,
        "read.cancel-attempt-locally",
        ReplaySafety::Idempotent,
        &retry_policy,
        options(Duration::from_secs(1)),
        move |_| {
            let owner = Arc::clone(&local_owner);
            async move {
                owner.cancel();
                std::future::pending::<Result<(), &'static str>>().await
            }
        },
        |_| panic!("a cancelled retry child must not be classified"),
    )
    .await;

    assert!(matches!(
        result,
        Err(RetryExecutionError::Interrupted {
            attempts: 1,
            reason: Interruption::Cancelled,
            last_error: None,
        })
    ));
    assert!(local_parent.check().is_ok());
}

#[tokio::test(start_paused = true)]
async fn local_cancellation_retains_but_does_not_classify_a_same_poll_error() {
    let parent = batter_core::operation::OperationOwner::new(Duration::from_secs(10))
        .unwrap()
        .into_context();
    let owner = Arc::new(parent.child(Duration::from_secs(10)).unwrap());
    let context = owner.context().clone();
    let factories = Arc::new(AtomicU32::new(0));
    let classifiers = Arc::new(AtomicU32::new(0));
    let result: Result<(), _> = retry::execute_with_options(
        &context,
        "read.cancel-before-return",
        ReplaySafety::Idempotent,
        &policy(2, Duration::from_millis(10)),
        options(Duration::from_secs(1)),
        {
            let factories = Arc::clone(&factories);
            move |_| {
                factories.fetch_add(1, Ordering::SeqCst);
                let owner = Arc::clone(&owner);
                async move {
                    owner.cancel();
                    Err("current")
                }
            }
        },
        {
            let classifiers = Arc::clone(&classifiers);
            move |_| {
                classifiers.fetch_add(1, Ordering::SeqCst);
                RetryDecision::Retry
            }
        },
    )
    .await;

    assert!(matches!(
        result,
        Err(RetryExecutionError::Interrupted {
            attempts: 1,
            reason: Interruption::Cancelled,
            last_error: Some("current"),
        })
    ));
    assert_eq!(factories.load(Ordering::SeqCst), 1);
    assert_eq!(classifiers.load(Ordering::SeqCst), 0);
    assert!(parent.check().is_ok());
}

#[tokio::test]
async fn cancellation_in_backoff_retains_error_without_next_factory() {
    let owner = batter_core::operation::OperationOwner::new(Duration::from_secs(10)).unwrap();
    let context = owner.context().clone();
    let owned = context.clone();
    let backoff_started = Arc::new(tokio::sync::Notify::new());
    let factories = Arc::new(AtomicU32::new(0));
    let task = tokio::spawn({
        let backoff_started = Arc::clone(&backoff_started);
        let factories = Arc::clone(&factories);
        async move {
            retry::execute_with_options(
                &owned,
                "read.backoff-cancel",
                ReplaySafety::Idempotent,
                &policy(3, Duration::from_secs(5)),
                options(Duration::from_secs(1)).with_jitter(move || {
                    backoff_started.notify_one();
                    0
                }),
                move |_| {
                    factories.fetch_add(1, Ordering::SeqCst);
                    async { Err::<(), _>("first") }
                },
                |_| RetryDecision::Retry,
            )
            .await
        }
    });
    backoff_started.notified().await;
    owner.cancel();
    let result = task.await.unwrap();

    assert!(matches!(
        result,
        Err(RetryExecutionError::Interrupted {
            attempts: 1,
            reason: Interruption::Cancelled,
            last_error: Some("first"),
        })
    ));
    assert_eq!(factories.load(Ordering::SeqCst), 1);
}

struct PendingAttempt {
    dropped: Arc<AtomicBool>,
}

impl Future for PendingAttempt {
    type Output = Result<(), &'static str>;

    fn poll(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Self::Output> {
        Poll::Pending
    }
}

impl Drop for PendingAttempt {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
    }
}

#[tokio::test(start_paused = true)]
async fn attempt_expiration_destroys_future_and_cancels_its_scope() {
    let retained_scope = Arc::new(Mutex::new(None));
    let dropped = Arc::new(AtomicBool::new(false));
    let result = retry::execute_with_options(
        &batter_core::operation::OperationOwner::new(Duration::from_secs(10))
            .unwrap()
            .into_context(),
        "read.destroy",
        ReplaySafety::Idempotent,
        &policy(2, Duration::from_millis(10)),
        options(Duration::from_millis(200)),
        {
            let retained_scope = Arc::clone(&retained_scope);
            let dropped = Arc::clone(&dropped);
            move |attempt| {
                *retained_scope.lock().unwrap() = Some(attempt.context);
                PendingAttempt {
                    dropped: Arc::clone(&dropped),
                }
            }
        },
        |_| panic!("a pending interrupted attempt must not be classified"),
    )
    .await;

    assert!(matches!(
        result,
        Err(RetryExecutionError::AttemptDeadlineExceeded {
            attempts: 1,
            last_error: None,
        })
    ));
    assert!(dropped.load(Ordering::SeqCst));
    assert_eq!(
        retained_scope.lock().unwrap().as_ref().unwrap().check(),
        Err(Interruption::Cancelled)
    );
}

#[tokio::test(start_paused = true)]
async fn jitter_and_provider_floor_compose_with_attempt_options() {
    let started = Instant::now();
    let mut draws = 0;
    let mut deadlines = Vec::new();
    let mut scopes = Vec::new();
    let result = retry::execute_with_options(
        &batter_core::operation::OperationOwner::new(Duration::from_secs(10))
            .unwrap()
            .into_context(),
        "read.jitter-options",
        ReplaySafety::Deduplicated,
        &policy(2, Duration::from_millis(100)),
        options(Duration::from_secs(1)).with_jitter(|| {
            draws += 1;
            0
        }),
        |attempt| {
            deadlines.push(attempt.context.deadline());
            scopes.push(attempt.context.clone());
            std::future::ready(if attempt.number == 1 {
                Err("busy")
            } else {
                Ok(42)
            })
        },
        |_| RetryDecision::RetryAfter(Duration::from_millis(200)),
    )
    .await;

    assert_eq!(result.unwrap(), 42);
    assert_eq!(draws, 1);
    assert_eq!(Instant::now() - started, Duration::from_millis(200));
    assert_eq!(
        deadlines,
        [
            started + Duration::from_secs(1),
            started + Duration::from_millis(1_200),
        ]
    );
    assert!(
        scopes
            .iter()
            .all(|scope| scope.check() == Err(Interruption::Cancelled))
    );
}

#[tokio::test]
async fn attempt_panic_propagates_without_classification() {
    let classifiers = Arc::new(AtomicU32::new(0));
    let task: tokio::task::JoinHandle<Result<(), RetryExecutionError<&'static str>>> =
        tokio::spawn({
            let classifiers = Arc::clone(&classifiers);
            async move {
                retry::execute_with_options(
                    &batter_core::operation::OperationOwner::new(Duration::from_secs(10))
                        .unwrap()
                        .into_context(),
                    "read.panic",
                    ReplaySafety::Idempotent,
                    &policy(2, Duration::from_millis(10)),
                    options(Duration::from_secs(1)),
                    |_| async { panic!("attempt panic") },
                    move |_| {
                        classifiers.fetch_add(1, Ordering::SeqCst);
                        RetryDecision::Retry
                    },
                )
                .await
            }
        });

    assert!(task.await.unwrap_err().is_panic());
    assert_eq!(classifiers.load(Ordering::SeqCst), 0);
}
