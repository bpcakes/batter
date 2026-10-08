//! Both startup acknowledgement policies, including their failure paths.

use super::support::*;
use batter_core::{
    lifecycle::{Readiness, ShutdownCause, ShutdownFailure},
    periodic::{
        PeriodicCompletion, PeriodicInitializationExpired, PeriodicShutdown, register_periodic_in,
    },
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::time::sleep;

#[tokio::test(start_paused = true)]
async fn immediate_acknowledgement_does_not_claim_that_maintenance_succeeded() {
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
    // Readiness is reached even though every run has failed.
    running.status().wait_ready().await.unwrap();
    sleep(Duration::from_millis(2_500)).await;
    assert!(reader.snapshot().acknowledged);
    assert_eq!(reader.snapshot().succeeded, 0);
    assert!(reader.snapshot().recoverable_failures >= 1);
    // Recoverable history alone does not make checked completion fail.
    let success = running.shutdown_checked().await.unwrap();
    let summary = &success.report().periodic[0].summary;
    assert!(summary.recoverable_failures >= 1);
    assert_eq!(summary.completion, PeriodicCompletion::Stopped);
}

#[tokio::test(start_paused = true)]
async fn the_first_run_is_admitted_while_the_process_is_still_starting() {
    let mut supervisor = supervisor();
    let reader = register_periodic_in(
        &mut supervisor,
        "storage.pruning",
        immediate(SECOND, SECOND, PeriodicShutdown::StopAtDrain),
        |_| async { succeeded() },
    )
    .unwrap();
    // Deliberately withholding application approval keeps admission Starting.
    let pending = supervisor.start_unapproved();
    sleep(Duration::from_millis(1_500)).await;
    assert_eq!(pending.status().readiness(), Readiness::Starting);
    assert!(reader.snapshot().succeeded >= 2);
    // Public readiness-gated admission is unchanged by that private path.
    assert_eq!(
        pending
            .operation_admission()
            .admit_root(batter_core::operation::RootDeadline::after(SECOND).unwrap())
            .unwrap_err(),
        Readiness::Starting,
    );
    let running = pending.approve_readiness();
    drop(running.shutdown_checked().await.unwrap());
}

#[tokio::test(start_paused = true)]
async fn first_success_acknowledges_only_after_a_run_succeeds() {
    let mut supervisor = supervisor();
    let attempts = Arc::new(AtomicUsize::new(0));
    let counted = attempts.clone();
    let reader = register_periodic_in(
        &mut supervisor,
        "lease.renewal",
        first_success(SECOND, SECOND, SECOND * 10, PeriodicShutdown::StopAtDrain),
        move |_| {
            let attempt = counted.fetch_add(1, Ordering::SeqCst) as u32;
            async move {
                // Two failures and a timeout still recover inside the allowance.
                match attempt {
                    0 | 1 => failed(attempt),
                    2 => {
                        sleep(SECOND * 2).await;
                        succeeded()
                    }
                    _ => succeeded(),
                }
            }
        },
    )
    .unwrap();
    let status = supervisor.status();
    let running = supervisor.start();
    assert_eq!(status.readiness(), Readiness::Starting);
    running.status().wait_ready().await.unwrap();
    let summary = reader.snapshot();
    assert!(summary.acknowledged);
    assert_eq!(summary.succeeded, 1);
    assert_eq!(summary.recoverable_failures, 2);
    assert_eq!(summary.deadline_exceeded, 1);
    assert_eq!(summary.invocations, 4);
    drop(running.shutdown_checked().await.unwrap());
}

#[tokio::test(start_paused = true)]
async fn an_expired_initialization_allowance_is_a_retained_failure_that_drains() {
    let mut supervisor = supervisor();
    let attempts = Arc::new(AtomicUsize::new(0));
    let counted = attempts.clone();
    let reader = register_periodic_in(
        &mut supervisor,
        "lease.renewal",
        first_success(
            SECOND,
            SECOND,
            Duration::from_millis(2_500),
            PeriodicShutdown::StopAtDrain,
        ),
        move |_| {
            let attempt = counted.fetch_add(1, Ordering::SeqCst) as u32;
            async move { failed(attempt) }
        },
    )
    .unwrap();
    let status = supervisor.status();
    let running = supervisor.start();
    let ShutdownFailure::Report(report) = running.wait_checked().await.unwrap_err() else {
        panic!("expected the initialization failure report")
    };
    assert_eq!(report.cause, ShutdownCause::ComponentExit("lease.renewal"));
    let retained = report.tasks[0]
        .error
        .as_ref()
        .expect("the initialization failure is retained");
    assert_eq!(
        retained
            .downcast_ref::<PeriodicInitializationExpired>()
            .expect("the library-owned initialization failure keeps its type")
            .name,
        "lease.renewal",
    );
    // Expiry prevented readiness and the three failed runs stay accounted for.
    assert_eq!(status.readiness(), Readiness::Stopped);
    let summary = &report.periodic[0].summary;
    assert!(!summary.acknowledged);
    assert_eq!(
        summary.completion,
        PeriodicCompletion::InitializationExpired
    );
    assert_eq!(summary.invocations, 3);
    assert_eq!(summary.recoverable_failures, 3);
    assert_eq!(reader.snapshot().invocations, 3);
}

/// Deliberately uses the real clock: a blocking poll is the documented way the
/// cooperative run boundary can return success after its deadline has passed.
#[tokio::test]
async fn a_success_arriving_after_the_allowance_cannot_acknowledge_startup() {
    let mut supervisor = supervisor();
    let runs = Arc::new(AtomicUsize::new(0));
    let counted = runs.clone();
    let reader = register_periodic_in(
        &mut supervisor,
        "lease.renewal",
        first_success(
            SECOND * 10,
            SECOND * 10,
            // Generous enough that the first run certainly starts inside it.
            Duration::from_millis(100),
            PeriodicShutdown::StopAtDrain,
        ),
        move |_| {
            counted.fetch_add(1, Ordering::SeqCst);
            async {
                // A poll that blocks its runtime thread cannot be preempted by
                // the run's own deadline, so this success is returned well
                // after the initialization allowance expired.
                std::thread::sleep(Duration::from_millis(400));
                succeeded()
            }
        },
    )
    .unwrap();
    let status = supervisor.status();
    let running = supervisor.start();
    let ShutdownFailure::Report(report) = running.wait_checked().await.unwrap_err() else {
        panic!("expected the initialization failure report")
    };
    // The run itself is preserved, but it could not acknowledge startup.
    assert_eq!(runs.load(Ordering::SeqCst), 1);
    let summary = &report.periodic[0].summary;
    assert_eq!(summary.succeeded, 1);
    assert_eq!(summary.invocations, 1);
    assert!(!summary.acknowledged);
    assert!(summary.has_failures());
    assert_eq!(
        summary.completion,
        PeriodicCompletion::InitializationExpired
    );
    assert_eq!(report.cause, ShutdownCause::ComponentExit("lease.renewal"));
    assert!(
        report.tasks[0]
            .error
            .as_ref()
            .unwrap()
            .downcast_ref::<PeriodicInitializationExpired>()
            .is_some()
    );
    // Expiry prevented readiness even though a run had succeeded.
    assert_eq!(status.readiness(), Readiness::Stopped);
    assert_eq!(reader.snapshot().succeeded, 1);
}

#[tokio::test(start_paused = true)]
async fn drain_during_pending_support_initialization_abandons_without_readiness() {
    let mut supervisor = supervisor();
    let entered = Arc::new(tokio::sync::Notify::new());
    let signalled = entered.clone();
    let reader = register_periodic_in(
        &mut supervisor,
        "lease.renewal",
        first_success(
            SECOND,
            SECOND * 100,
            SECOND * 100,
            PeriodicShutdown::SupportThroughDrain,
        ),
        move |_| {
            let signalled = signalled.clone();
            async move {
                signalled.notify_one();
                std::future::pending::<()>().await;
                succeeded()
            }
        },
    )
    .unwrap();
    let status = supervisor.status();
    let running = supervisor.start();
    entered.notified().await;
    assert_eq!(status.readiness(), Readiness::Starting);
    // Abandoning a never-initialized support component is an expected exit.
    let success = running.shutdown_checked().await.unwrap();
    let summary = &success.report().periodic[0].summary;
    assert!(!summary.acknowledged);
    assert_eq!(
        summary.completion,
        PeriodicCompletion::AbandonedDuringStartup
    );
    assert_eq!(summary.stop_interrupted, 1);
    assert_eq!(reader.snapshot().succeeded, 0);
}

#[tokio::test(start_paused = true)]
async fn later_failures_never_revoke_an_issued_acknowledgement() {
    let mut supervisor = supervisor();
    let attempts = Arc::new(AtomicUsize::new(0));
    let counted = attempts.clone();
    let reader = register_periodic_in(
        &mut supervisor,
        "lease.renewal",
        first_success(SECOND, SECOND, SECOND * 10, PeriodicShutdown::StopAtDrain),
        move |_| {
            let attempt = counted.fetch_add(1, Ordering::SeqCst) as u32;
            async move {
                if attempt == 0 {
                    succeeded()
                } else {
                    failed(attempt)
                }
            }
        },
    )
    .unwrap();
    let running = supervisor.start();
    running.status().wait_ready().await.unwrap();
    sleep(Duration::from_millis(3_500)).await;
    assert_eq!(running.status().readiness(), Readiness::Ready);
    assert!(reader.snapshot().recoverable_failures >= 3);
    drop(running.shutdown_checked().await.unwrap());
}
