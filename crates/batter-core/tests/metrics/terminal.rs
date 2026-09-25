//! Terminal results that finish without a boundary value: whole retry
//! executions, abandoned admission waits and abandoned cleanup hooks.

use super::{assert_catalog, capture::Capture, owner};
use batter_core::{
    admission::{Admission, Bulkhead, BulkheadCapacity},
    cleanup::{CleanupBudget, CleanupStack},
    lifecycle::Supervisor,
    operation::OperationContext,
    operation::RootDeadline,
    retry::{self, ReplaySafety, RetryDecision, RetryError, RetryPolicy, StopReason},
    telemetry::metrics::*,
};
use std::time::Duration;

async fn retry_once(
    context: &OperationContext,
    decision: RetryDecision,
) -> RetryError<&'static str> {
    let policy = RetryPolicy::new(2, Duration::from_millis(1), Duration::from_millis(1)).unwrap();
    retry::execute(
        context,
        "provider.write",
        ReplaySafety::Idempotent,
        &policy,
        |_| async { Err::<(), _>("rejected") },
        move |_| decision,
    )
    .await
    .unwrap_err()
}

#[tokio::test]
async fn every_retry_execution_records_one_terminal_result() {
    let capture = Capture::unbounded();
    let _recorder = facade::set_default_local_recorder(&capture);
    let context = owner().into_context();

    let stopped = retry_once(&context, RetryDecision::Stop).await;
    assert!(matches!(
        stopped,
        RetryError::Stopped {
            reason: StopReason::NotRetryable,
            ..
        }
    ));
    let exhausted = retry_once(&context, RetryDecision::Retry).await;
    assert!(matches!(
        exhausted,
        RetryError::Stopped {
            reason: StopReason::AttemptsExhausted,
            ..
        }
    ));
    let cancelled_owner = owner();
    cancelled_owner.cancel();
    let cancelled = retry_once(cancelled_owner.context(), RetryDecision::Retry).await;
    assert!(matches!(
        cancelled,
        RetryError::Interrupted { attempts: 0, .. }
    ));
    let dropped = tokio::time::timeout(
        Duration::from_millis(1),
        retry::execute(
            &context,
            "provider.write",
            ReplaySafety::Idempotent,
            &RetryPolicy::new(1, Duration::from_millis(1), Duration::from_millis(1)).unwrap(),
            |_| std::future::pending::<Result<(), &str>>(),
            |_| RetryDecision::Stop,
        ),
    )
    .await;
    assert!(dropped.is_err());

    for result in [
        "not_retryable",
        "attempts_exhausted",
        "cancelled",
        "dropped",
    ] {
        let labels = [("operation", "provider.write"), ("result", result)];
        assert_eq!(capture.count(RETRY_EXECUTIONS, &labels), 1.0, "{result}");
    }
    assert_eq!(capture.samples(RETRY_EXECUTIONS).len(), 4);
    assert_eq!(capture.samples(OPERATION_COMPLETIONS), []);
    assert_catalog(&capture);
}

#[tokio::test]
async fn abandoned_admission_wait_records_a_dropped_decision() {
    let capture = Capture::unbounded();
    let _recorder = facade::set_default_local_recorder(&capture);
    let context = owner().into_context();
    let bulkhead = Bulkhead::new(BulkheadCapacity::new(1).unwrap());
    let _held = bulkhead.enter(&context, Admission::Reject).await.unwrap();
    let waiting = tokio::time::timeout(
        Duration::from_millis(1),
        bulkhead.enter(&context, Admission::Wait),
    )
    .await;
    assert!(waiting.is_err());
    let decision = |decision| {
        capture.count(
            ADMISSION_DECISIONS,
            &[("admission", "bulkhead"), ("decision", decision)],
        )
    };
    assert_eq!(decision("admitted"), 1.0);
    assert_eq!(decision("dropped"), 1.0);
    assert_eq!(capture.samples(OPERATION_COMPLETIONS), []);
    assert_catalog(&capture);
}

#[tokio::test]
async fn abandoned_cleanup_hooks_are_counted_as_dropped() {
    let capture = Capture::unbounded();
    let _recorder = facade::set_default_local_recorder(&capture);
    let mut unclosed = CleanupStack::new();
    unclosed.push("first.close", || async { Ok(()) }).unwrap();
    unclosed.push("second.close", || async { Ok(()) }).unwrap();
    drop(unclosed);

    let second = Duration::from_secs(1);
    let budget = CleanupBudget::new(second, second, second).unwrap();
    let mut in_flight = CleanupStack::new();
    in_flight
        .push("pending.close", || {
            std::future::pending::<Result<(), batter_core::BoxError>>()
        })
        .unwrap();
    let closing = tokio::time::timeout(Duration::from_millis(1), in_flight.close(budget)).await;
    assert!(closing.is_err());

    assert_eq!(capture.count(CLEANUP_HOOKS, &[("outcome", "dropped")]), 3.0);
    assert_catalog(&capture);
}

#[tokio::test]
async fn root_lifecycle_admission_records_each_observed_state() {
    let capture = Capture::unbounded();
    let _recorder = facade::set_default_local_recorder(&capture);
    let mut supervisor = Supervisor::new(super::shutdown_budget());
    supervisor
        .register("service.component", |startup| async move {
            let shutdown = startup.acknowledge_started();
            shutdown.draining().await;
            Ok(shutdown.stopped())
        })
        .unwrap();
    let admission = supervisor.operation_admission();
    let deadline = || RootDeadline::after(Duration::from_secs(1)).unwrap();
    assert!(admission.admit_root(deadline()).is_err());
    let handle = supervisor.handle();
    let running = supervisor.start();
    handle.status().wait_ready().await.unwrap();
    for _ in 0..3 {
        assert!(admission.admit_root(deadline()).is_ok());
    }
    running.shutdown_checked().await.unwrap();
    assert!(admission.admit_root(deadline()).is_err());
    let root = |decision| {
        capture.count(
            ADMISSION_DECISIONS,
            &[("admission", "root"), ("decision", decision)],
        )
    };
    assert_eq!(root("starting"), 1.0);
    assert_eq!(root("admitted"), 3.0);
    assert_eq!(root("stopped"), 1.0);
    assert_catalog(&capture);
}
