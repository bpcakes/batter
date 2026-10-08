//! Exhaustive run outcomes and the bounded evidence they produce.

use super::support::*;
use batter_core::{
    lifecycle::{Readiness, ShutdownCause, ShutdownFailure, TaskOutcome},
    periodic::{PeriodicCompletion, PeriodicShutdown, register_periodic_in},
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::time::{Instant, sleep, timeout};

#[tokio::test(start_paused = true)]
async fn a_recoverable_failure_waits_for_the_next_scheduled_invocation() {
    let schedule = Schedule::default();
    let mut supervisor = supervisor();
    let recorded = schedule.clone();
    let origin = Instant::now();
    let attempts = Arc::new(AtomicUsize::new(0));
    register_periodic_in(
        &mut supervisor,
        "storage.pruning",
        immediate(SECOND, SECOND, PeriodicShutdown::StopAtDrain),
        move |_| {
            let started = recorded.started(origin);
            let attempt = attempts.fetch_add(1, Ordering::SeqCst) as u32;
            async move {
                drop(started);
                failed(attempt)
            }
        },
    )
    .unwrap();
    let running = supervisor.start();
    running.status().wait_ready().await.unwrap();
    sleep(Duration::from_millis(2_500)).await;
    // No extra immediate retry: exactly one invocation per interval.
    assert_eq!(schedule.starts(), vec![Duration::ZERO, SECOND, SECOND * 2]);
    assert_eq!(running.status().readiness(), Readiness::Ready);
    drop(running.shutdown_checked().await.unwrap());
}

#[tokio::test(start_paused = true)]
async fn many_recoverable_failures_keep_bounded_samples_and_exact_counters() {
    let mut supervisor = supervisor();
    let attempts = Arc::new(AtomicUsize::new(0));
    let counted = attempts.clone();
    let reader = register_periodic_in(
        &mut supervisor,
        "storage.pruning",
        immediate(SECOND, SECOND, PeriodicShutdown::StopAtDrain),
        move |_| {
            let attempt = counted.fetch_add(1, Ordering::SeqCst) as u32;
            async move {
                // Five failures, then recovery without losing the history.
                if attempt < 5 {
                    failed(attempt)
                } else {
                    succeeded()
                }
            }
        },
    )
    .unwrap();
    let running = supervisor.start();
    running.status().wait_ready().await.unwrap();
    sleep(Duration::from_millis(6_500)).await;
    let success = running.shutdown_checked().await.unwrap();
    let summary = &success.report().periodic[0].summary;
    assert_eq!(summary.invocations, 7);
    assert_eq!(summary.recoverable_failures, 5);
    assert_eq!(summary.succeeded, 2);
    assert_eq!(summary.deadline_exceeded, 0);
    assert_eq!(summary.stop_interrupted, 0);
    assert!(!summary.saturated);
    // Two bounded samples, three explicitly unsampled, nothing counted twice.
    assert_eq!(summary.unsampled_failures, 3);
    let first = summary.first_failure.as_ref().unwrap();
    let last = summary.last_failure.as_ref().unwrap();
    assert_eq!((first.invocation, last.invocation), (1, 5));
    assert_eq!(first.error.downcast_ref::<Attempt>().unwrap().0, 0);
    assert_eq!(last.error.downcast_ref::<Attempt>().unwrap().0, 4);
    assert_eq!(reader.snapshot().unsampled_failures, 3);
}

#[tokio::test(start_paused = true)]
async fn only_an_explicit_escalation_initiates_drain() {
    let mut supervisor = supervisor();
    let attempts = Arc::new(AtomicUsize::new(0));
    let counted = attempts.clone();
    register_periodic_in(
        &mut supervisor,
        "storage.pruning",
        immediate(SECOND, SECOND, PeriodicShutdown::StopAtDrain),
        move |_| {
            let attempt = counted.fetch_add(1, Ordering::SeqCst) as u32;
            async move {
                if attempt < 3 {
                    failed(attempt)
                } else {
                    escalated(attempt)
                }
            }
        },
    )
    .unwrap();
    let status = supervisor.status();
    let running = supervisor.start();
    running.status().wait_ready().await.unwrap();
    let ShutdownFailure::Report(report) = running.wait_checked().await.unwrap_err() else {
        panic!("expected the escalated run report")
    };
    // The three ordinary failures never drained the process.
    assert_eq!(status.readiness(), Readiness::Stopped);
    assert_eq!(
        report.cause,
        ShutdownCause::ComponentExit("storage.pruning")
    );
    assert_eq!(report.tasks[0].outcome, TaskOutcome::Failed);
    // The terminal cause is shared with the retained history, so the task
    // record exposes the original error through its source chain.
    assert_eq!(
        report.tasks[0]
            .error
            .as_ref()
            .and_then(|error| error.source())
            .and_then(|source| source.downcast_ref::<Attempt>())
            .map(|attempt| attempt.0),
        Some(3),
    );
    let summary = &report.periodic[0].summary;
    let terminal = summary
        .terminal_failure
        .as_ref()
        .expect("the escalated cause is retained independently of the task");
    assert_eq!(terminal.invocation, 4);
    assert_eq!(terminal.error.downcast_ref::<Attempt>().unwrap().0, 3);
    assert_eq!(summary.completion, PeriodicCompletion::Fatal);
    assert_eq!(summary.invocations, 4);
    assert_eq!(summary.recoverable_failures, 3);
}

#[tokio::test(start_paused = true)]
async fn earlier_history_survives_a_panicking_run_without_a_final_snapshot() {
    let mut supervisor = supervisor();
    let attempts = Arc::new(AtomicUsize::new(0));
    let counted = attempts.clone();
    let reader = register_periodic_in(
        &mut supervisor,
        "storage.pruning",
        immediate(SECOND, SECOND, PeriodicShutdown::StopAtDrain),
        move |_| {
            let attempt = counted.fetch_add(1, Ordering::SeqCst) as u32;
            async move {
                if attempt < 2 {
                    failed(attempt)
                } else {
                    panic!("maintenance run panicked")
                }
            }
        },
    )
    .unwrap();
    let running = supervisor.start();
    running.status().wait_ready().await.unwrap();
    let ShutdownFailure::Report(report) = running.wait_checked().await.unwrap_err() else {
        panic!("expected the panicked component report")
    };
    assert_eq!(report.tasks[0].outcome, TaskOutcome::Panicked);
    let summary = &report.periodic[0].summary;
    // The runner published no final snapshot, so the evidence is incomplete,
    // yet the earlier failures and their concrete causes are still retained.
    assert!(!summary.is_complete());
    assert_eq!(summary.completion, PeriodicCompletion::Pending);
    assert_eq!(summary.recoverable_failures, 2);
    assert_eq!(summary.invocations, 3);
    assert_eq!(reader.snapshot().recoverable_failures, 2);
}

#[tokio::test(start_paused = true)]
async fn history_survives_a_lost_driver_waiter() {
    let mut supervisor = supervisor();
    let attempts = Arc::new(AtomicUsize::new(0));
    let counted = attempts.clone();
    let reader = register_periodic_in(
        &mut supervisor,
        "storage.pruning",
        immediate(SECOND, SECOND, PeriodicShutdown::StopAtDrain),
        move |_| {
            let attempt = counted.fetch_add(1, Ordering::SeqCst) as u32;
            async move { failed(attempt) }
        },
    )
    .unwrap();
    // Abandoning the caller-owned driver destroys the runner without a report.
    assert!(
        timeout(
            Duration::from_millis(2_500),
            supervisor.run_until(std::future::pending()),
        )
        .await
        .is_err()
    );
    let summary = reader.snapshot();
    assert_eq!(summary.recoverable_failures, 3);
    assert_eq!(summary.completion, PeriodicCompletion::Pending);
    assert_eq!(summary.first_failure.unwrap().invocation, 1);
}

/// Panics while destroying a periodic factory's captured value.
struct PanicOnDrop;

impl Drop for PanicOnDrop {
    fn drop(&mut self) {
        panic!("captured maintenance state panicked while being destroyed")
    }
}

#[tokio::test(start_paused = true)]
async fn an_escalated_cause_survives_a_panicking_capture_destructor() {
    let mut supervisor = supervisor();
    let witness = PanicOnDrop;
    register_periodic_in(
        &mut supervisor,
        "storage.pruning",
        immediate(SECOND, SECOND, PeriodicShutdown::StopAtDrain),
        move |_| {
            // The witness belongs to the factory, so it is destroyed with the
            // component future, after the run has already escalated.
            let _ = &witness;
            async { escalated(9) }
        },
    )
    .unwrap();
    supervisor
        .on_cleanup("dependency", || async {
            panic!("a panicked component must block dependency cleanup")
        })
        .unwrap();
    let running = supervisor.start();
    let ShutdownFailure::Report(report) = running.wait_checked().await.unwrap_err() else {
        panic!("expected the panicked component report")
    };
    // The task's own returned error was discarded by the unwind, so the report
    // retains only the panic for that task.
    assert_eq!(report.tasks[0].outcome, TaskOutcome::Panicked);
    assert!(
        report.tasks[0]
            .error
            .as_ref()
            .and_then(|error| error.source())
            .and_then(|source| source.downcast_ref::<Attempt>())
            .is_none()
    );
    // The escalated cause survives independently of that task result.
    let summary = &report.periodic[0].summary;
    assert_eq!(summary.completion, PeriodicCompletion::Fatal);
    assert!(summary.has_failures());
    let terminal = summary
        .terminal_failure
        .as_ref()
        .expect("the escalated cause is retained before the factory is destroyed");
    assert_eq!(terminal.invocation, 1);
    assert_eq!(terminal.error.downcast_ref::<Attempt>().unwrap().0, 9);
    // Conservative cleanup skipping is unchanged by that retention.
    assert_eq!(
        report.cleanup.skipped[0].reason,
        batter_core::cleanup::SkipReason::UnsafeTaskExit,
    );
    assert!(!format!("{report:?}").contains("maintenance attempt failed"));
}

/// Escalates immediately, then panics when the enclosing boundary destroys it.
struct EscalateThenPanicOnDrop(u32);

impl std::future::Future for EscalateThenPanicOnDrop {
    type Output = Run;

    fn poll(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        std::task::Poll::Ready(escalated(self.0))
    }
}

impl Drop for EscalateThenPanicOnDrop {
    fn drop(&mut self) {
        panic!("maintenance run future panicked while being destroyed")
    }
}

#[tokio::test(start_paused = true)]
async fn an_escalated_cause_survives_a_panicking_run_future_destructor() {
    let mut supervisor = supervisor();
    register_periodic_in(
        &mut supervisor,
        "storage.pruning",
        immediate(SECOND, SECOND, PeriodicShutdown::StopAtDrain),
        |_| EscalateThenPanicOnDrop(11),
    )
    .unwrap();
    supervisor
        .on_cleanup("dependency", || async {
            panic!("a panicked component must block dependency cleanup")
        })
        .unwrap();
    let running = supervisor.start();
    let ShutdownFailure::Report(report) = running.wait_checked().await.unwrap_err() else {
        panic!("expected the panicked component report")
    };
    assert_eq!(report.tasks[0].outcome, TaskOutcome::Panicked);
    let summary = &report.periodic[0].summary;
    // The run future was destroyed before the loop could publish anything, so
    // the snapshot is explicitly incomplete.
    assert_eq!(summary.completion, PeriodicCompletion::Pending);
    assert!(!summary.is_complete());
    // The escalated cause was retained while that future was still alive, and
    // retained terminal evidence is reported independently of the marker.
    let terminal = summary
        .terminal_failure
        .as_ref()
        .expect("the escalated cause is retained before the run future dies");
    assert_eq!(terminal.invocation, 1);
    assert_eq!(terminal.error.downcast_ref::<Attempt>().unwrap().0, 11);
    assert!(summary.has_failures());
    assert_eq!(summary.recoverable_failures, 0);
    // Conservative cleanup skipping is unchanged by that retention.
    assert_eq!(
        report.cleanup.skipped[0].reason,
        batter_core::cleanup::SkipReason::UnsafeTaskExit,
    );
    assert!(!format!("{report:?}").contains("maintenance attempt failed"));
}

#[tokio::test(start_paused = true)]
async fn report_formatting_never_reveals_a_retained_cause() {
    let mut supervisor = supervisor();
    let attempts = Arc::new(AtomicUsize::new(0));
    let counted = attempts.clone();
    let reader = register_periodic_in(
        &mut supervisor,
        "storage.pruning",
        immediate(SECOND, SECOND, PeriodicShutdown::StopAtDrain),
        move |_| {
            let attempt = counted.fetch_add(1, Ordering::SeqCst) as u32;
            async move { failed(attempt) }
        },
    )
    .unwrap();
    let running = supervisor.start();
    running.status().wait_ready().await.unwrap();
    sleep(Duration::from_millis(1_500)).await;
    let success = running.shutdown_checked().await.unwrap();
    let report = success.report();
    let rendered = format!("{report:?} {report} {:?}", reader.snapshot());
    assert!(
        !rendered.contains("maintenance attempt failed"),
        "{rendered}"
    );
    assert!(rendered.contains("retained"), "{rendered}");
    // The concrete cause stays reachable through the explicit field.
    assert!(
        report.periodic[0]
            .summary
            .first_failure
            .as_ref()
            .unwrap()
            .error
            .downcast_ref::<Attempt>()
            .is_some()
    );
}
