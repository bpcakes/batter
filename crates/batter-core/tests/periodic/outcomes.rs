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
    assert_eq!(
        report.tasks[0]
            .error
            .as_ref()
            .and_then(|error| error.downcast_ref::<Attempt>())
            .map(|attempt| attempt.0),
        Some(3),
    );
    let summary = &report.periodic[0].summary;
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
