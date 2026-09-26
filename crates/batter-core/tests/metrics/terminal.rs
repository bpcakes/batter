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

    // Started attempts only: 1 + 2 returned, none for the pre-cancelled
    // execution, and 1 dropped with its execution.
    assert_eq!(capture.samples(RETRY_ATTEMPTS).len(), 4);
    assert_eq!(
        capture.count(RETRY_ATTEMPTS, &[("outcome", "dropped")]),
        1.0
    );
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

    // Never-run hooks of the unclosed stack are `dropped`; the hook the close
    // driver had taken is `abandoned`, because it may have started.
    assert_eq!(capture.count(CLEANUP_HOOKS, &[("outcome", "dropped")]), 2.0);
    assert_eq!(
        capture.count(CLEANUP_HOOKS, &[("outcome", "abandoned")]),
        1.0
    );
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

#[tokio::test]
async fn unwinding_boundaries_record_panicked_instead_of_dropped() {
    let capture = Capture::unbounded();
    let _recorder = facade::set_default_local_recorder(&capture);
    let context = owner().into_context();
    let operation = tokio::spawn({
        let context = context.clone();
        async move {
            context
                .run("example.read", |_| async {
                    panic!("operation defect");
                    #[allow(unreachable_code)]
                    Ok::<(), &str>(())
                })
                .await
        }
    });
    assert!(operation.await.unwrap_err().is_panic());
    let retry = tokio::spawn(async move {
        let policy =
            RetryPolicy::new(2, Duration::from_millis(1), Duration::from_millis(1)).unwrap();
        retry::execute(
            &context,
            "provider.write",
            ReplaySafety::Idempotent,
            &policy,
            |_| async {
                panic!("attempt defect");
                #[allow(unreachable_code)]
                Ok::<(), &str>(())
            },
            |_| RetryDecision::Retry,
        )
        .await
    });
    assert!(retry.await.unwrap_err().is_panic());

    let operation = [("operation", "example.read"), ("outcome", "panicked")];
    assert_eq!(capture.count(OPERATION_COMPLETIONS, &operation), 1.0);
    let attempt = [("operation", "provider.write"), ("outcome", "panicked")];
    assert_eq!(capture.count(RETRY_ATTEMPTS, &attempt), 1.0);
    let execution = [("operation", "provider.write"), ("result", "panicked")];
    assert_eq!(capture.count(RETRY_EXECUTIONS, &execution), 1.0);
    assert_eq!(
        capture.count(OPERATION_COMPLETIONS, &[("outcome", "dropped")]),
        0.0
    );
    assert_catalog(&capture);
}

fn stubborn_supervisor() -> Supervisor {
    let mut supervisor = Supervisor::new(super::shutdown_budget());
    supervisor
        .on_cleanup("dependency.close", || async { Ok(()) })
        .unwrap();
    supervisor
        .register("service.component", |startup| async move {
            let _shutdown = startup.acknowledge_started();
            // Ignores drain so the driver stays inside its drain phase.
            std::future::pending::<()>().await;
            unreachable!()
        })
        .unwrap();
    supervisor
}

#[tokio::test]
async fn abandoned_shutdown_drivers_record_exactly_one_dropped_shutdown() {
    let capture = Capture::unbounded();
    let _recorder = facade::set_default_local_recorder(&capture);

    let running = stubborn_supervisor().run_until(std::future::pending());
    assert!(
        tokio::time::timeout(Duration::from_millis(20), running)
            .await
            .is_err()
    );
    let draining = stubborn_supervisor().run_until(std::future::ready(()));
    assert!(
        tokio::time::timeout(Duration::from_millis(20), draining)
            .await
            .is_err()
    );

    let shutdowns = |cause| capture.count(SHUTDOWNS, &[("cause", cause), ("result", "dropped")]);
    assert_eq!(shutdowns("none"), 1.0);
    assert_eq!(shutdowns("requested"), 1.0);
    assert_eq!(capture.samples(SHUTDOWNS).len(), 2);
    // Each abandoned supervisor's never-run cleanup is recorded before its
    // shutdown sample, which is the last record of that drive.
    let cleanup = capture
        .first(CLEANUP_HOOKS, &[("outcome", "dropped")])
        .unwrap();
    assert!(cleanup < capture.first(SHUTDOWNS, &[]).unwrap());
    assert_eq!(capture.count(CLEANUP_HOOKS, &[("outcome", "dropped")]), 2.0);
    // Abandoned drivers produce no report, so they record no duration.
    assert_eq!(capture.samples(SHUTDOWN_DURATION), []);
    assert_catalog(&capture);
}

#[tokio::test]
async fn admission_wait_destroyed_while_unwinding_records_panicked() {
    let capture = Capture::unbounded();
    let _recorder = facade::set_default_local_recorder(&capture);
    let bulkhead = Bulkhead::new(BulkheadCapacity::new(1).unwrap());
    let context = owner().into_context();
    let _held = bulkhead.enter(&context, Admission::Reject).await.unwrap();
    let waiting = tokio::spawn({
        let bulkhead = bulkhead.clone();
        async move {
            let wait = bulkhead.enter(&context, Admission::Wait);
            tokio::pin!(wait);
            // A zero timeout polls the wait exactly once while it stays owned.
            let polled = tokio::time::timeout(Duration::ZERO, wait.as_mut()).await;
            assert!(polled.is_err());
            panic!("sibling defect");
        }
    });
    assert!(waiting.await.unwrap_err().is_panic());
    let decision = |decision| {
        capture.count(
            ADMISSION_DECISIONS,
            &[("admission", "bulkhead"), ("decision", decision)],
        )
    };
    assert_eq!(decision("panicked"), 1.0);
    assert_eq!(decision("dropped"), 0.0);
    assert_catalog(&capture);
}

#[test]
fn skipped_cleanup_is_counted_once_when_capture_destruction_panics() {
    let capture = Capture::unbounded();
    let _recorder = facade::set_default_local_recorder(&capture);
    struct PanicOnDrop;
    impl Drop for PanicOnDrop {
        fn drop(&mut self) {
            panic!("capture destruction failed");
        }
    }
    let mut stack = CleanupStack::new();
    stack.push("first.close", || async { Ok(()) }).unwrap();
    let captured = PanicOnDrop;
    stack
        .push("second.close", move || async move {
            drop(captured);
            Ok(())
        })
        .unwrap();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        stack.skip(batter_core::cleanup::SkipReason::UnsafeTaskExit)
    }));
    assert!(result.is_err());
    assert_eq!(capture.count(CLEANUP_HOOKS, &[("outcome", "skipped")]), 2.0);
    assert_eq!(capture.count(CLEANUP_HOOKS, &[("outcome", "dropped")]), 0.0);
    assert_eq!(capture.count(CLEANUP_HOOKS, &[]), 2.0);
    assert_catalog(&capture);
}

#[tokio::test(start_paused = true)]
async fn shutdown_duration_uses_native_clock_tightened_after_drain_starts() {
    use batter_core::lifecycle::{ManagedComponent, ManagedSettlement};
    struct Settled;
    impl ManagedSettlement for Settled {
        fn is_success(&self) -> bool {
            true
        }
        fn allows_dependency_cleanup(&self) -> bool {
            true
        }
    }

    let capture = Capture::unbounded();
    let _recorder = facade::set_default_local_recorder(&capture);
    let native_started = tokio::time::Instant::now();
    let phase = Duration::from_secs(10);
    let cleanup = CleanupBudget::new(phase, phase, phase).unwrap();
    let budget = batter_core::lifecycle::ShutdownBudget::new(phase, phase, phase, cleanup).unwrap();
    let mut supervisor = Supervisor::new(budget);
    let (stop, stopped) = tokio::sync::oneshot::channel();
    supervisor
        .register_managed("native", owner().into_context(), move |_| {
            let mut stop = Some(stop);
            Ok(ManagedComponent::new(
                async { Ok(()) },
                std::future::pending(),
                move |parent_started| {
                    assert!(parent_started >= native_started);
                    if let Some(stop) = stop.take() {
                        stop.send(()).unwrap();
                    }
                    native_started
                },
                async move {
                    stopped.await.unwrap();
                    Settled
                },
            ))
        })
        .unwrap();
    let running = supervisor.start();
    running.status().wait_ready().await.unwrap();
    tokio::time::advance(Duration::from_secs(5)).await;
    running.shutdown_checked().await.unwrap();
    assert_eq!(capture.samples(SHUTDOWN_DURATION).len(), 1);
    assert_eq!(capture.count(SHUTDOWN_DURATION, &[]), 5.0);
    assert_eq!(capture.count(SHUTDOWNS, &[]), 1.0);
    assert_catalog(&capture);
}
