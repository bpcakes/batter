use batter::{
    admission::{Admission, AdmissionError, Bulkhead},
    operation::{Interruption, OperationContext},
    retry::{self, ReplaySafety, RetryDecision, RetryError, RetryPolicy, StopReason},
};
use batter_test_support::Script;
use std::{
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
    time::Duration,
};
use tokio::time::Instant;

fn assert_elapsed(actual: Duration, expected: Duration) {
    // Tokio timers have millisecond granularity; do not assert sub-tick equality.
    assert!(actual >= expected && actual <= expected + Duration::from_millis(10));
}

fn policy(attempts: u32) -> RetryPolicy {
    RetryPolicy::new(
        attempts,
        Duration::from_millis(100),
        Duration::from_millis(200),
    )
    .unwrap()
}
fn context() -> OperationContext {
    OperationContext::new(Duration::from_secs(10)).unwrap()
}

#[test]
fn invalid_policy_is_rejected() {
    assert!(RetryPolicy::new(0, Duration::from_secs(1), Duration::from_secs(1)).is_err());
    assert!(RetryPolicy::new(1, Duration::ZERO, Duration::from_secs(1)).is_err());
    assert!(RetryPolicy::new(2, Duration::from_secs(2), Duration::from_secs(1)).is_err());
}

#[tokio::test(start_paused = true)]
async fn fresh_attempts_eventually_succeed() {
    let script = Script::new([Err("first"), Err("second"), Ok(42)]);
    let started = Instant::now();
    let result = retry::execute(
        &context(),
        "read",
        ReplaySafety::Idempotent,
        &policy(3),
        |_| {
            let script = script.clone();
            async move { script.next() }
        },
        |_| RetryDecision::Retry,
    )
    .await
    .unwrap();
    assert_eq!(result, 42);
    assert_eq!(script.calls(), 3);
    assert_elapsed(started.elapsed(), Duration::from_millis(300));
}

#[tokio::test(start_paused = true)]
async fn never_replays_and_does_not_call_classifier() {
    let script: Script<(), _> = Script::new([Err("write")]);
    let result = retry::execute(
        &context(),
        "write",
        ReplaySafety::Never,
        &policy(3),
        |_| {
            let script = script.clone();
            async move { script.next() }
        },
        |_| panic!("replay is forbidden before classification"),
    )
    .await;
    assert!(matches!(
        result,
        Err(RetryError::Stopped {
            attempts: 1,
            reason: StopReason::ReplayForbidden,
            ..
        })
    ));
    assert_eq!(script.calls(), 1);
}

#[tokio::test(start_paused = true)]
async fn attempt_limit_counts_initial_attempt() {
    let result: Result<(), _> = retry::execute(
        &context(),
        "exhausted",
        ReplaySafety::Idempotent,
        &policy(3),
        |_| async { Err("latest") },
        |_| RetryDecision::Retry,
    )
    .await;
    assert!(matches!(
        result,
        Err(RetryError::Stopped {
            attempts: 3,
            reason: StopReason::AttemptsExhausted,
            error: "latest"
        })
    ));
}

#[tokio::test(start_paused = true)]
async fn classification_can_stop_immediately() {
    let result: Result<(), _> = retry::execute(
        &context(),
        "terminal",
        ReplaySafety::Idempotent,
        &policy(3),
        |_| async { Err("invalid") },
        |_| RetryDecision::Stop,
    )
    .await;
    assert!(matches!(
        result,
        Err(RetryError::Stopped {
            attempts: 1,
            reason: StopReason::NotRetryable,
            ..
        })
    ));
}

#[tokio::test(start_paused = true)]
async fn provider_delay_is_a_lower_bound() {
    let script = Script::new([Err("busy"), Ok(())]);
    let started = Instant::now();
    retry::execute(
        &context(),
        "provider",
        ReplaySafety::Deduplicated,
        &policy(2),
        |_| {
            let script = script.clone();
            async move { script.next() }
        },
        |_| RetryDecision::RetryAfter(Duration::from_secs(2)),
    )
    .await
    .unwrap();
    assert_elapsed(started.elapsed(), Duration::from_secs(2));
}

#[tokio::test(start_paused = true)]
async fn too_long_provider_delay_does_not_get_shortened() {
    let context = OperationContext::new(Duration::from_millis(50)).unwrap();
    let result: Result<(), _> = retry::execute(
        &context,
        "budget",
        ReplaySafety::Idempotent,
        &policy(3),
        |_| async { Err("busy") },
        |_| RetryDecision::RetryAfter(Duration::from_secs(1)),
    )
    .await;
    assert!(matches!(
        result,
        Err(RetryError::Stopped {
            attempts: 1,
            reason: StopReason::InsufficientBudget,
            error: "busy"
        })
    ));
    assert!(context.check().is_ok()); // Not falsely reported as an elapsed deadline.
}

#[tokio::test(start_paused = true)]
async fn cancelled_before_first_attempt_starts_nothing() {
    let context = context();
    context.cancel();
    let calls = Arc::new(AtomicU32::new(0));
    let result: Result<(), _> = retry::execute(
        &context,
        "preflight",
        ReplaySafety::Idempotent,
        &policy(3),
        |_| {
            calls.fetch_add(1, Ordering::SeqCst);
            async { Err("no") }
        },
        |_| RetryDecision::Retry,
    )
    .await;
    assert!(matches!(
        result,
        Err(RetryError::Interrupted {
            attempts: 0,
            reason: Interruption::Cancelled,
            last_error: None
        })
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn cancellation_in_backoff_retains_last_error() {
    let context = context();
    let owned = context.clone();
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        let mut sender = Some(sender);
        let long_backoff =
            RetryPolicy::new(3, Duration::from_secs(5), Duration::from_secs(5)).unwrap();
        retry::execute(
            &owned,
            "backoff",
            ReplaySafety::Idempotent,
            &long_backoff,
            |_| {
                if let Some(sender) = sender.take() {
                    sender.send(()).unwrap();
                }
                async { Err::<(), _>("first") }
            },
            |_| RetryDecision::Retry,
        )
        .await
    });
    receiver.await.unwrap();
    context.cancel();
    let result = task.await.unwrap();
    assert!(matches!(
        result,
        Err(RetryError::Interrupted {
            attempts: 1,
            reason: Interruption::Cancelled,
            last_error: Some("first")
        })
    ));
}

#[tokio::test(start_paused = true)]
async fn attempt_timeout_is_not_automatically_retried() {
    let context = OperationContext::new(Duration::from_secs(1)).unwrap();
    let result: Result<(), _> = retry::execute(
        &context,
        "timeout",
        ReplaySafety::Idempotent,
        &policy(3),
        |_| async {
            tokio::time::sleep(Duration::from_secs(2)).await;
            Err("late")
        },
        |_| RetryDecision::Retry,
    )
    .await;
    assert!(matches!(
        result,
        Err(RetryError::Interrupted {
            attempts: 1,
            reason: Interruption::DeadlineExceeded,
            last_error: None
        })
    ));
}

#[tokio::test(start_paused = true)]
async fn later_attempt_interruption_preserves_previous_failure() {
    let context = OperationContext::new(Duration::from_millis(200)).unwrap();
    let result: Result<(), _> = retry::execute(
        &context,
        "second-timeout",
        ReplaySafety::Idempotent,
        &policy(3),
        |attempt| async move {
            if attempt.number > 1 {
                tokio::time::sleep(Duration::from_secs(10)).await;
            }
            Err("first")
        },
        |_| RetryDecision::Retry,
    )
    .await;
    assert!(matches!(
        result,
        Err(RetryError::Interrupted {
            attempts: 2,
            reason: Interruption::DeadlineExceeded,
            last_error: Some("first")
        })
    ));
}

#[tokio::test(start_paused = true)]
async fn backoff_caps_without_resetting_the_total_context() {
    let context = context();
    let deadline = context.deadline();
    let started = Instant::now();
    let _: Result<(), _> = retry::execute(
        &context,
        "capped",
        ReplaySafety::Idempotent,
        &policy(4),
        |attempt| async move {
            assert_eq!(attempt.context.deadline(), deadline);
            Err("retry")
        },
        |_| RetryDecision::Retry,
    )
    .await;
    assert_elapsed(started.elapsed(), Duration::from_millis(500));
}

async fn jittered_schedule(mut seed: u64) -> Vec<Duration> {
    let mut starts = Vec::new();
    let result: Result<(), _> = retry::execute_with_jitter(
        &context(),
        "seeded",
        ReplaySafety::Idempotent,
        &policy(5),
        move || {
            // An explicitly seeded test stream; production supplies its own RNG.
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            seed
        },
        |_| {
            starts.push(Instant::now());
            async { Err("retry") }
        },
        |_| RetryDecision::Retry,
    )
    .await;
    assert!(matches!(
        result,
        Err(RetryError::Stopped {
            attempts: 5,
            reason: StopReason::AttemptsExhausted,
            ..
        })
    ));
    starts.windows(2).map(|pair| pair[1] - pair[0]).collect()
}

#[tokio::test(start_paused = true)]
async fn seeded_jitter_is_reproducible_and_distinct_streams_spread_retries() {
    let (first, repeated, independent) = tokio::join!(
        jittered_schedule(42),
        jittered_schedule(42),
        jittered_schedule(43),
    );
    assert_eq!(first, repeated);
    assert_ne!(first, independent);
    for schedule in [first, independent] {
        for (index, delay) in schedule.into_iter().enumerate() {
            let cap = Duration::from_millis(if index == 0 { 100 } else { 200 });
            assert!(delay >= cap / 2 && delay <= cap + Duration::from_millis(10));
        }
    }
}

#[tokio::test(start_paused = true)]
async fn jitter_endpoints_stay_positive_and_within_exponential_cap() {
    for (sample, expected) in [(0, 250), (u64::MAX, 500)] {
        let started = Instant::now();
        let mut draws = 0;
        let result: Result<(), _> = retry::execute_with_jitter(
            &context(),
            "jitter.endpoint",
            ReplaySafety::Idempotent,
            &policy(4),
            || {
                draws += 1;
                sample
            },
            |_| async { Err("retry") },
            |_| RetryDecision::Retry,
        )
        .await;
        assert!(matches!(
            result,
            Err(RetryError::Stopped { attempts: 4, .. })
        ));
        assert_eq!(draws, 3);
        assert_elapsed(started.elapsed(), Duration::from_millis(expected));
    }

    let started = Instant::now();
    let tiny = RetryPolicy::new(2, Duration::from_nanos(1), Duration::from_nanos(1)).unwrap();
    let _: Result<(), _> = retry::execute_with_jitter(
        &context(),
        "jitter.minimum",
        ReplaySafety::Idempotent,
        &tiny,
        || 0,
        |_| async { Err("retry") },
        |_| RetryDecision::Retry,
    )
    .await;
    assert_elapsed(started.elapsed(), Duration::from_nanos(1));
}

#[tokio::test(start_paused = true)]
async fn jitter_never_shortens_provider_delay_even_beyond_the_policy_cap() {
    let started = Instant::now();
    let result = retry::execute_with_jitter(
        &context(),
        "jitter.provider",
        ReplaySafety::Deduplicated,
        &policy(2),
        || 0,
        |attempt| async move {
            if attempt.number == 1 {
                Err("busy")
            } else {
                Ok(42)
            }
        },
        |_| RetryDecision::RetryAfter(Duration::from_secs(2)),
    )
    .await;
    assert_eq!(result.unwrap(), 42);
    assert_elapsed(started.elapsed(), Duration::from_secs(2));
}

#[tokio::test(start_paused = true)]
async fn retry_and_admission_share_work_budget_and_preserve_finalization() {
    let parent = OperationContext::new(Duration::from_secs(1)).unwrap();
    let phases = parent
        .reserve_finalization(Duration::from_millis(300))
        .unwrap();
    let work_deadline = phases.work().deadline();
    let bulkhead = Bulkhead::new(1).unwrap();
    let _occupied = bulkhead.enter(&parent, Admission::Reject).await.unwrap();
    let result: Result<(), _> = retry::execute_with_jitter(
        phases.work(),
        "jitter.reserved",
        ReplaySafety::Idempotent,
        &policy(3),
        || 0,
        |attempt| {
            let bulkhead = bulkhead.clone();
            async move {
                if attempt.number == 1 {
                    return Err("first");
                }
                assert_eq!(attempt.context.deadline(), work_deadline);
                match bulkhead.enter(&attempt.context, Admission::Wait).await {
                    Err(AdmissionError::Interrupted(_)) => Err("admission interrupted"),
                    other => panic!("unexpected admission: {other:?}"),
                }
            }
        },
        |_| RetryDecision::Retry,
    )
    .await;
    assert!(matches!(
        result,
        Err(RetryError::Interrupted {
            attempts: 2,
            reason: Interruption::DeadlineExceeded,
            last_error: Some("first"),
        })
    ));
    phases
        .finalization()
        .run("reserved.finalize", |_| async {
            tokio::time::sleep(Duration::from_millis(200)).await;
            Ok::<_, std::io::Error>(())
        })
        .await
        .unwrap();
    assert!(parent.check().is_ok());
}

#[tokio::test(start_paused = true)]
async fn jitter_provider_wait_cannot_consume_finalization_reserve() {
    let parent = OperationContext::new(Duration::from_secs(1)).unwrap();
    let phases = parent
        .reserve_finalization(Duration::from_millis(500))
        .unwrap();
    let result: Result<(), _> = retry::execute_with_jitter(
        phases.work(),
        "jitter.budget",
        ReplaySafety::Idempotent,
        &policy(3),
        || 0,
        |_| async { Err("busy") },
        |_| RetryDecision::RetryAfter(Duration::from_millis(600)),
    )
    .await;
    assert!(matches!(
        result,
        Err(RetryError::Stopped {
            attempts: 1,
            reason: StopReason::InsufficientBudget,
            error: "busy",
        })
    ));
    assert!(phases.work().check().is_ok());
    assert_eq!(phases.finalization().remaining(), Duration::from_secs(1));
}

#[tokio::test(start_paused = true)]
async fn forbidden_replay_never_samples_jitter() {
    let result: Result<(), _> = retry::execute_with_jitter(
        &context(),
        "jitter.forbidden",
        ReplaySafety::Never,
        &policy(3),
        || panic!("forbidden retries must not sample jitter"),
        |_| async { Err("write") },
        |_| panic!("forbidden retries must not classify"),
    )
    .await;
    assert!(matches!(
        result,
        Err(RetryError::Stopped {
            reason: StopReason::ReplayForbidden,
            ..
        })
    ));
}
