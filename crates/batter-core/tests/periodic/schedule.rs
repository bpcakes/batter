//! Immediate-first, serial, missed-tick-skipping cadence and per-run budgets.

use super::support::*;
use batter_core::{
    operation::{Interruption, OperationContext},
    periodic::{PeriodicShutdown, register_periodic_in},
};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::time::{Instant, sleep};

#[tokio::test(start_paused = true)]
async fn short_intervals_skip_overdue_ticks_even_inside_five_milliseconds() {
    short_interval_overrun(false).await;
}

#[tokio::test(start_paused = true)]
async fn fast_recoverable_failures_do_not_replay_short_interval_ticks() {
    short_interval_overrun(true).await;
}

async fn short_interval_overrun(recoverable: bool) {
    for period in [Duration::from_millis(1), Duration::from_micros(250)] {
        let schedule = Schedule::default();
        let recorded = schedule.clone();
        let mut supervisor = supervisor();
        let entered = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let signal = entered.clone();
        let resume = release.clone();
        let origin = Instant::now();
        let mut invocation = 0;
        register_periodic_in(
            &mut supervisor,
            "storage.pruning",
            immediate(period, SECOND, PeriodicShutdown::StopAtDrain),
            move |_| {
                let active = recorded.started(origin);
                let (signal, resume) = (signal.clone(), resume.clone());
                invocation += 1;
                let attempt = invocation;
                async move {
                    if attempt == 1 {
                        signal.notify_one();
                        resume.notified().await;
                    }
                    drop(active);
                    if recoverable {
                        failed(attempt)
                    } else {
                        succeeded()
                    }
                }
            },
        )
        .unwrap();
        let running = supervisor.start();
        entered.notified().await;
        // Hold the first invocation across several deadlines, all within
        // Tokio's five-ms missed-tick tolerance, then let fast runs proceed.
        tokio::time::advance(Duration::from_millis(4)).await;
        release.notify_one();
        for _ in 0..16 {
            tokio::task::yield_now().await;
        }
        assert_eq!(
            schedule.starts(),
            [Duration::ZERO, Duration::from_millis(4)]
        );
        tokio::time::advance(Duration::from_millis(1)).await;
        for _ in 0..16 {
            tokio::task::yield_now().await;
        }
        // Sub-ms deadlines share Tokio's timer resolution, but still cannot
        // turn a single wakeup into a burst of overdue invocations.
        assert_eq!(
            schedule.starts(),
            [
                Duration::ZERO,
                Duration::from_millis(4),
                Duration::from_millis(5)
            ]
        );
        let report = running.shutdown_checked().await.unwrap();
        let summary = &report.report().periodic[0].summary;
        assert_eq!(summary.invocations, 3);
        assert_eq!(
            summary.recoverable_failures,
            if recoverable { 3 } else { 0 }
        );
        assert_eq!(schedule.overlapped(), 0);
    }
}

#[tokio::test(start_paused = true)]
async fn the_first_invocation_is_immediate_and_later_ones_follow_the_interval() {
    let schedule = Schedule::default();
    let mut supervisor = supervisor();
    let recorded = schedule.clone();
    let origin = Instant::now();
    register_periodic_in(
        &mut supervisor,
        "storage.pruning",
        immediate(SECOND, SECOND, PeriodicShutdown::StopAtDrain),
        move |_| {
            let started = recorded.started(origin);
            async move {
                drop(started);
                succeeded()
            }
        },
    )
    .unwrap();
    let running = supervisor.start();
    running.status().wait_ready().await.unwrap();
    sleep(Duration::from_millis(3_500)).await;
    let success = running.shutdown_checked().await.unwrap();
    assert_eq!(
        schedule.starts(),
        vec![
            Duration::ZERO,
            SECOND,
            SECOND * 2,
            SECOND * 3,
            // Drain itself destroys the loop, so no later tick is admitted.
        ],
    );
    assert_eq!(schedule.overlapped(), 0);
    assert_eq!(success.report().periodic[0].summary.succeeded, 4);
}

#[tokio::test(start_paused = true)]
async fn an_overrun_never_overlaps_and_replays_no_burst_of_missed_work() {
    let schedule = Schedule::default();
    let mut supervisor = supervisor();
    let recorded = schedule.clone();
    let origin = Instant::now();
    register_periodic_in(
        &mut supervisor,
        "storage.pruning",
        immediate(SECOND, SECOND * 10, PeriodicShutdown::StopAtDrain),
        move |_| {
            let started = recorded.started(origin);
            async move {
                // Each run takes two and a half intervals.
                sleep(Duration::from_millis(2_500)).await;
                drop(started);
                succeeded()
            }
        },
    )
    .unwrap();
    let running = supervisor.start();
    running.status().wait_ready().await.unwrap();
    sleep(Duration::from_millis(9_000)).await;
    drop(running.shutdown_checked().await.unwrap());
    // Four serial runs in nine seconds, not nine queued ticks.
    assert_eq!(
        schedule.starts(),
        vec![
            Duration::ZERO,
            Duration::from_millis(2_500),
            Duration::from_millis(5_000),
            Duration::from_millis(7_500),
        ],
    );
    assert_eq!(schedule.overlapped(), 0);
}

#[tokio::test(start_paused = true)]
async fn one_long_pause_produces_exactly_one_overdue_invocation() {
    let schedule = Schedule::default();
    let mut supervisor = supervisor();
    let recorded = schedule.clone();
    let origin = Instant::now();
    let first = Arc::new(AtomicUsize::new(0));
    register_periodic_in(
        &mut supervisor,
        "storage.pruning",
        immediate(SECOND, SECOND * 10, PeriodicShutdown::StopAtDrain),
        move |_| {
            let started = recorded.started(origin);
            let attempt = first.fetch_add(1, Ordering::SeqCst);
            async move {
                if attempt == 0 {
                    // One long pause spans five and a half intervals.
                    sleep(Duration::from_millis(5_500)).await;
                }
                drop(started);
                succeeded()
            }
        },
    )
    .unwrap();
    let running = supervisor.start();
    running.status().wait_ready().await.unwrap();
    sleep(Duration::from_millis(8_500)).await;
    drop(running.shutdown_checked().await.unwrap());
    assert_eq!(
        schedule.starts(),
        vec![
            Duration::ZERO,
            // Exactly one overdue invocation, then the schedule realigns.
            Duration::from_millis(5_500),
            Duration::from_millis(6_000),
            Duration::from_millis(7_000),
            Duration::from_millis(8_000),
        ],
    );
}

#[tokio::test(start_paused = true)]
async fn a_budget_longer_than_the_interval_bounds_one_run_without_overlapping() {
    let schedule = Schedule::default();
    let mut supervisor = supervisor();
    let recorded = schedule.clone();
    let origin = Instant::now();
    register_periodic_in(
        &mut supervisor,
        "storage.pruning",
        immediate(SECOND, SECOND * 3, PeriodicShutdown::StopAtDrain),
        move |_| {
            let started = recorded.started(origin);
            async move {
                sleep(Duration::from_millis(2_000)).await;
                drop(started);
                succeeded()
            }
        },
    )
    .unwrap();
    let running = supervisor.start();
    running.status().wait_ready().await.unwrap();
    sleep(Duration::from_millis(4_500)).await;
    let success = running.shutdown_checked().await.unwrap();
    let summary = &success.report().periodic[0].summary;
    // Two-second runs fit inside a three-second budget.
    assert_eq!(summary.deadline_exceeded, 0);
    assert!(summary.succeeded >= 2);
    assert_eq!(schedule.overlapped(), 0);
}

#[tokio::test(start_paused = true)]
async fn each_run_gets_a_fresh_budget_after_an_expired_one() {
    let mut supervisor = supervisor();
    let attempts = Arc::new(AtomicUsize::new(0));
    let remaining = Arc::new(Mutex::new(Vec::new()));
    let observed = remaining.clone();
    register_periodic_in(
        &mut supervisor,
        "storage.pruning",
        immediate(SECOND * 4, SECOND, PeriodicShutdown::StopAtDrain),
        move |scope: OperationContext| {
            let attempt = attempts.fetch_add(1, Ordering::SeqCst);
            let observed = observed.clone();
            async move {
                observed.lock().unwrap().push(scope.remaining());
                if attempt == 0 {
                    // Exhaust the first run's own budget.
                    sleep(SECOND * 2).await;
                }
                succeeded()
            }
        },
    )
    .unwrap();
    let running = supervisor.start();
    running.status().wait_ready().await.unwrap();
    sleep(Duration::from_millis(5_000)).await;
    let success = running.shutdown_checked().await.unwrap();
    let summary = &success.report().periodic[0].summary;
    assert_eq!(summary.deadline_exceeded, 1);
    assert_eq!(summary.succeeded, 1);
    // The later run started from a complete budget, not the expired remainder.
    assert_eq!(*remaining.lock().unwrap(), vec![SECOND, SECOND]);
}

#[tokio::test(start_paused = true)]
async fn a_child_scope_cannot_outlive_its_own_run() {
    let mut supervisor = supervisor();
    let observed = Arc::new(Mutex::new(Vec::new()));
    let recorded = observed.clone();
    register_periodic_in(
        &mut supervisor,
        "storage.pruning",
        immediate(SECOND * 10, SECOND, PeriodicShutdown::StopAtDrain),
        move |scope: OperationContext| {
            let recorded = recorded.clone();
            async move {
                // A child budget far beyond the run's own cannot extend it.
                let Ok(child) = scope.child(SECOND * 600) else {
                    return escalated(0);
                };
                recorded
                    .lock()
                    .unwrap()
                    .push((child.context().deadline() == scope.deadline(), {
                        // Cancelling this run's child cannot cancel the run.
                        child.cancel();
                        scope.check()
                    }));
                succeeded()
            }
        },
    )
    .unwrap();
    let running = supervisor.start();
    running.status().wait_ready().await.unwrap();
    sleep(Duration::from_millis(100)).await;
    drop(running.shutdown_checked().await.unwrap());
    assert_eq!(*observed.lock().unwrap(), vec![(true, Ok(()))]);
}

#[tokio::test(start_paused = true)]
async fn a_destroyed_run_is_recorded_once_as_a_stop_interruption() {
    let mut supervisor = supervisor();
    let started = Arc::new(tokio::sync::Notify::new());
    let entered = started.clone();
    register_periodic_in(
        &mut supervisor,
        "storage.pruning",
        immediate(SECOND, SECOND * 100, PeriodicShutdown::StopAtDrain),
        move |_: OperationContext| {
            let entered = entered.clone();
            async move {
                entered.notify_one();
                std::future::pending::<()>().await;
                succeeded()
            }
        },
    )
    .unwrap();
    let running = supervisor.start();
    running.status().wait_ready().await.unwrap();
    started.notified().await;
    let success = running.shutdown_checked().await.unwrap();
    let summary = &success.report().periodic[0].summary;
    assert_eq!(summary.invocations, 1);
    assert_eq!(summary.stop_interrupted, 1);
    assert_eq!(summary.succeeded, 0);
    assert_eq!(summary.deadline_exceeded, 0);
    assert_eq!(
        Interruption::Cancelled.to_string(),
        "operation cancelled; external outcome may be unknown",
    );
}
