//! Experimental wakeable scheduling: paused-time observable failure contracts.

use super::support::*;
use batter_core::{
    ConfigurationError, RegistrationError,
    lifecycle::Readiness,
    periodic::{PeriodicCompletion, PeriodicShutdown, WakeablePolicy, register_wakeable_in},
};
use std::{sync::Arc, time::Duration};
use tokio::{
    sync::Notify,
    time::{Instant, advance},
};

pub(super) async fn settle() {
    for _ in 0..16 {
        tokio::task::yield_now().await;
    }
}

pub(super) async fn elapse(duration: Duration) {
    advance(duration).await;
    settle().await;
}

fn policy(interval: Duration, spacing: Duration) -> WakeablePolicy {
    WakeablePolicy::new(
        immediate(interval, SECOND * 20, PeriodicShutdown::StopAtDrain),
        spacing,
    )
    .unwrap()
}

#[test]
fn spacing_is_validated_before_registration() {
    let base = immediate(SECOND, SECOND, PeriodicShutdown::StopAtDrain);
    assert_eq!(
        WakeablePolicy::new(base, Duration::ZERO),
        Err(ConfigurationError::Zero("periodic wake spacing"))
    );
    assert_eq!(
        WakeablePolicy::new(base, Duration::MAX),
        Err(ConfigurationError::TooLarge("periodic wake spacing"))
    );
    assert_eq!(policy(SECOND, SECOND * 2).wake_spacing(), SECOND * 2);
}

#[tokio::test(start_paused = true)]
async fn registration_and_wakes_are_inert_until_start_and_after_abandonment() {
    fn unstarted_work(_: batter_core::operation::OperationContext) -> std::future::Ready<Run> {
        panic!("an unstarted or rejected component cannot invoke work")
    }
    let mut supervisor = supervisor();
    let (reader, wake) = register_wakeable_in(
        &mut supervisor,
        "storage.refresh",
        policy(SECOND * 10, SECOND),
        unstarted_work,
    )
    .unwrap();
    wake.wake();
    assert_eq!(reader.snapshot().invocations, 0);
    let duplicate = register_wakeable_in(
        &mut supervisor,
        "storage.refresh",
        policy(SECOND * 10, SECOND),
        unstarted_work,
    );
    assert!(matches!(
        duplicate,
        Err(RegistrationError::Duplicate("storage.refresh"))
    ));
    let invalid = register_wakeable_in(
        &mut supervisor,
        "",
        policy(SECOND * 10, SECOND),
        |_| async { succeeded() },
    );
    assert!(matches!(invalid, Err(RegistrationError::InvalidName)));
    drop(supervisor);
    wake.wake();
    elapse(SECOND * 30).await;
    assert_eq!(reader.snapshot().invocations, 0);
}

#[tokio::test(start_paused = true)]
async fn wake_bursts_coalesce_enforce_spacing_and_count_every_admitted_run() {
    let mut supervisor = supervisor();
    let schedule = Schedule::default();
    let recorded = schedule.clone();
    let origin = Instant::now();
    let (reader, wake) = register_wakeable_in(
        &mut supervisor,
        "storage.refresh",
        policy(SECOND * 30, SECOND * 3),
        move |_| {
            let active = recorded.started(origin);
            async move {
                drop(active);
                succeeded()
            }
        },
    )
    .unwrap();
    // Pre-start requests coalesce with the immediate first run.
    for _ in 0..100 {
        wake.wake();
    }
    let running = supervisor.start();
    running.status().wait_ready().await.unwrap();
    settle().await;
    assert_eq!(schedule.starts(), [Duration::ZERO]);
    for _ in 0..100 {
        wake.clone().wake();
    }
    elapse(SECOND * 2).await;
    assert_eq!(schedule.count(), 1);
    elapse(SECOND).await;
    assert_eq!(schedule.starts(), [Duration::ZERO, SECOND * 3]);
    elapse(SECOND * 3).await;
    assert_eq!(schedule.count(), 2); // No queued wake burst.
    wake.wake();
    settle().await;
    assert_eq!(schedule.starts(), [Duration::ZERO, SECOND * 3, SECOND * 6]);
    let report = running.shutdown_checked().await.unwrap();
    assert_eq!(report.report().periodic[0].summary.invocations, 3);
    assert_eq!(reader.snapshot().succeeded, 3);
    assert_eq!(schedule.overlapped(), 0);
}

#[tokio::test(start_paused = true)]
async fn a_tick_inside_spacing_is_deferred_and_original_phase_is_preserved() {
    let mut supervisor = supervisor();
    let schedule = Schedule::default();
    let recorded = schedule.clone();
    let origin = Instant::now();
    let (_, wake) = register_wakeable_in(
        &mut supervisor,
        "storage.refresh",
        policy(SECOND * 10, SECOND * 4),
        move |_| {
            let active = recorded.started(origin);
            async move {
                drop(active);
                succeeded()
            }
        },
    )
    .unwrap();
    let running = supervisor.start();
    running.status().wait_ready().await.unwrap();
    elapse(SECOND * 8).await;
    wake.wake();
    settle().await;
    elapse(SECOND * 2).await; // Original tick at 10 is inside 8..12 spacing.
    assert_eq!(schedule.starts(), [Duration::ZERO, SECOND * 8]);
    elapse(SECOND * 2).await;
    assert_eq!(schedule.starts(), [Duration::ZERO, SECOND * 8, SECOND * 12]);
    elapse(SECOND * 7).await;
    assert_eq!(schedule.count(), 3);
    elapse(SECOND).await; // Original phase 20, not wake phase 18 or delayed phase 22.
    assert_eq!(
        schedule.starts(),
        [Duration::ZERO, SECOND * 8, SECOND * 12, SECOND * 20]
    );
    running.shutdown_checked().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn spacing_longer_than_interval_defers_ticks_without_needing_a_wake() {
    let mut supervisor = supervisor();
    let schedule = Schedule::default();
    let recorded = schedule.clone();
    let origin = Instant::now();
    let (_, wake) = register_wakeable_in(
        &mut supervisor,
        "storage.refresh",
        policy(SECOND * 2, SECOND * 5),
        move |_| {
            let active = recorded.started(origin);
            async move {
                drop(active);
                succeeded()
            }
        },
    )
    .unwrap();
    drop(wake); // No producer is needed for cadence progress.
    let running = supervisor.start();
    running.status().wait_ready().await.unwrap();
    elapse(SECOND * 4).await;
    assert_eq!(schedule.count(), 1);
    elapse(SECOND).await;
    elapse(SECOND * 5).await;
    assert_eq!(schedule.starts(), [Duration::ZERO, SECOND * 5, SECOND * 10]);
    running.shutdown_checked().await.unwrap();
}

#[derive(Clone, Copy)]
enum Step {
    First,
    Later,
}

#[tokio::test(start_paused = true)]
async fn wakes_during_an_active_run_are_retained_once_without_overlap_or_tick_burst() {
    let mut supervisor = supervisor();
    let schedule = Schedule::default();
    let recorded = schedule.clone();
    let release = Arc::new(Notify::new());
    let resume = release.clone();
    let origin = Instant::now();
    let mut step = Step::First;
    let (_, wake) = register_wakeable_in(
        &mut supervisor,
        "storage.refresh",
        policy(SECOND * 2, SECOND),
        move |_| {
            let active = recorded.started(origin);
            let step = std::mem::replace(&mut step, Step::Later);
            let resume = resume.clone();
            async move {
                if let Step::First = step {
                    resume.notified().await;
                }
                drop(active);
                succeeded()
            }
        },
    )
    .unwrap();
    let running = supervisor.start();
    running.status().wait_ready().await.unwrap();
    for _ in 0..100 {
        wake.wake();
    }
    elapse(SECOND * 7).await;
    assert_eq!(schedule.count(), 1);
    release.notify_one();
    settle().await;
    assert_eq!(schedule.starts(), [Duration::ZERO, SECOND * 7]);
    elapse(SECOND).await;
    assert_eq!(schedule.starts(), [Duration::ZERO, SECOND * 7, SECOND * 8]);
    assert_eq!(schedule.overlapped(), 0);
    running.shutdown_checked().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn initialization_wake_survives_a_failed_run_and_can_acknowledge_within_allowance() {
    for shutdown in [
        PeriodicShutdown::StopAtDrain,
        PeriodicShutdown::SupportThroughDrain,
    ] {
        let mut supervisor = supervisor();
        let release = Arc::new(Notify::new());
        let resume = release.clone();
        let mut step = Step::First;
        let (reader, wake) = register_wakeable_in(
            &mut supervisor,
            "storage.refresh",
            WakeablePolicy::new(
                first_success(SECOND * 30, SECOND * 10, SECOND * 5, shutdown),
                SECOND * 2,
            )
            .unwrap(),
            move |_| {
                let step = std::mem::replace(&mut step, Step::Later);
                let resume = resume.clone();
                async move {
                    match step {
                        Step::First => {
                            resume.notified().await;
                            failed(1)
                        }
                        Step::Later => succeeded(),
                    }
                }
            },
        )
        .unwrap();
        let running = supervisor.start();
        settle().await;
        assert_eq!(reader.snapshot().invocations, 1);
        assert_eq!(running.status().readiness(), Readiness::Starting);
        wake.wake();
        release.notify_one();
        settle().await;
        elapse(SECOND).await;
        assert!(!reader.snapshot().acknowledged);
        elapse(SECOND).await;
        running.status().wait_ready().await.unwrap();
        let summary = reader.snapshot();
        assert_eq!(summary.invocations, 2);
        assert_eq!(summary.recoverable_failures, 1);
        assert_eq!(summary.succeeded, 1);
        running.shutdown_checked().await.unwrap();
    }
}

#[tokio::test(start_paused = true)]
async fn initialization_allowance_wins_over_a_pending_wake_at_the_spacing_boundary() {
    for shutdown in [
        PeriodicShutdown::StopAtDrain,
        PeriodicShutdown::SupportThroughDrain,
    ] {
        let mut supervisor = supervisor();
        let (reader, wake) = register_wakeable_in(
            &mut supervisor,
            "storage.refresh",
            WakeablePolicy::new(
                first_success(SECOND * 30, SECOND, SECOND * 2, shutdown),
                SECOND * 2,
            )
            .unwrap(),
            |_| async { failed(1) },
        )
        .unwrap();
        let running = supervisor.start();
        settle().await;
        wake.wake();
        elapse(SECOND * 2).await;
        let report = running.wait().await.unwrap();
        assert!(!report.is_success());
        let summary = reader.snapshot();
        assert_eq!(summary.invocations, 1);
        assert!(!summary.acknowledged);
        assert_eq!(
            summary.completion,
            PeriodicCompletion::InitializationExpired
        );
        wake.wake();
        elapse(SECOND * 30).await;
        assert_eq!(reader.snapshot().invocations, 1);
    }
}

#[tokio::test(start_paused = true)]
async fn wake_admitted_initialization_run_keeps_the_original_allowance_and_budget() {
    let mut supervisor = supervisor();
    let origin = Instant::now();
    let mut step = Step::First;
    let (reader, wake) = register_wakeable_in(
        &mut supervisor,
        "storage.refresh",
        WakeablePolicy::new(
            first_success(
                SECOND * 30,
                SECOND * 10,
                SECOND * 3,
                PeriodicShutdown::StopAtDrain,
            ),
            SECOND,
        )
        .unwrap(),
        move |scope| {
            let step = std::mem::replace(&mut step, Step::Later);
            async move {
                match step {
                    Step::First => failed(1),
                    Step::Later => {
                        assert_eq!(scope.deadline(), origin + SECOND * 3);
                        std::future::pending::<()>().await;
                        succeeded()
                    }
                }
            }
        },
    )
    .unwrap();
    let running = supervisor.start();
    settle().await;
    wake.wake();
    elapse(SECOND).await;
    assert_eq!(reader.snapshot().invocations, 2);
    let report = running.wait().await.unwrap();
    assert!(!report.is_success());
    assert_eq!(Instant::now(), origin + SECOND * 3);
    let summary = reader.snapshot();
    assert_eq!(summary.deadline_exceeded, 1);
    assert_eq!(
        summary.completion,
        PeriodicCompletion::InitializationExpired
    );
}

#[tokio::test(start_paused = true)]
async fn drain_rejects_pending_wakes_and_post_end_wakes_are_noops() {
    for shutdown in [
        PeriodicShutdown::StopAtDrain,
        PeriodicShutdown::SupportThroughDrain,
    ] {
        let mut supervisor = supervisor();
        let (reader, wake) = register_wakeable_in(
            &mut supervisor,
            "storage.refresh",
            WakeablePolicy::new(immediate(SECOND * 30, SECOND, shutdown), SECOND).unwrap(),
            |_| async { succeeded() },
        )
        .unwrap();
        let running = supervisor.start();
        running.status().wait_ready().await.unwrap();
        settle().await;
        wake.wake();
        running.handle().request();
        wake.wake();
        running.wait_checked().await.unwrap();
        assert_eq!(reader.snapshot().completion, PeriodicCompletion::Stopped);
        for _ in 0..100 {
            wake.wake();
        }
        elapse(SECOND * 60).await;
        assert_eq!(reader.snapshot().invocations, 1);
    }
}
