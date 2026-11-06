//! Wakes use the existing admission, stop, cleanup and outcome boundaries.

use super::{
    support::*,
    wake::{elapse, settle},
};
use batter_core::{
    lifecycle::Readiness,
    periodic::{PeriodicCompletion, PeriodicShutdown, WakeablePolicy, register_wakeable_in},
};
use std::{sync::Arc, time::Duration};
use tokio::{sync::Notify, time::Instant};

#[derive(Clone, Copy)]
enum Step {
    First,
    Later,
}

#[tokio::test(start_paused = true)]
async fn wake_admitted_support_run_is_capped_and_destroyed_before_cleanup() {
    let mut supervisor = supervisor_with(SECOND * 10);
    let release = Arc::new(Notify::new());
    let resume = release.clone();
    supervisor
        .register("worker", move |startup| async move {
            let running = startup.acknowledge_started();
            running.draining().await;
            resume.notified().await;
            Ok(running.stopped())
        })
        .unwrap();
    let order = Order::default();
    let observed = order.clone();
    let origin = Instant::now();
    let mut step = Step::First;
    let (reader, wake) = register_wakeable_in(
        &mut supervisor,
        "storage.refresh",
        WakeablePolicy::new(
            immediate(
                SECOND * 30,
                SECOND * 100,
                PeriodicShutdown::SupportThroughDrain,
            ),
            SECOND,
        )
        .unwrap(),
        move |scope| {
            let step = std::mem::replace(&mut step, Step::Later);
            let observed = observed.clone();
            async move {
                match step {
                    Step::First => succeeded(),
                    Step::Later => {
                        assert_eq!(scope.deadline(), origin + SECOND * 12);
                        let guard = Destroyed(observed, "run.destroyed");
                        std::future::pending::<()>().await;
                        drop(guard);
                        succeeded()
                    }
                }
            }
        },
    )
    .unwrap();
    let closing = order.clone();
    let closing_wake = wake.clone();
    supervisor
        .on_cleanup("dependency", move || async move {
            closing_wake.wake();
            closing.push("cleanup");
            Ok(())
        })
        .unwrap();
    let running = supervisor.start();
    running.status().wait_ready().await.unwrap();
    elapse(SECOND * 2).await;
    running.handle().request();
    wake.wake();
    settle().await;
    assert_eq!(reader.snapshot().invocations, 2);
    assert!(order.observed().is_empty());
    wake.wake(); // Pending work cannot prolong support beyond ordinary settlement.
    release.notify_one();
    let report = running.wait_checked().await.unwrap();
    assert_eq!(Instant::now(), origin + SECOND * 2);
    assert_eq!(order.observed(), ["run.destroyed", "cleanup"]);
    assert_eq!(report.report().periodic[0].summary.invocations, 2);
    assert_eq!(reader.snapshot().stop_interrupted, 1);
    wake.wake();
    elapse(SECOND * 40).await;
    assert_eq!(reader.snapshot().invocations, 2);
}

#[tokio::test(start_paused = true)]
async fn drain_abandons_initialization_despite_a_pending_wake_and_open_support_boundary() {
    let mut supervisor = supervisor_with(SECOND * 10);
    let release = Arc::new(Notify::new());
    let resume = release.clone();
    supervisor
        .register("worker", move |startup| async move {
            let running = startup.acknowledge_started();
            running.draining().await;
            resume.notified().await;
            Ok(running.stopped())
        })
        .unwrap();
    let (reader, wake) = register_wakeable_in(
        &mut supervisor,
        "storage.refresh",
        WakeablePolicy::new(
            first_success(
                SECOND * 30,
                SECOND,
                SECOND * 10,
                PeriodicShutdown::SupportThroughDrain,
            ),
            SECOND * 2,
        )
        .unwrap(),
        |_| async { failed(1) },
    )
    .unwrap();
    let running = supervisor.start();
    settle().await;
    assert_eq!(running.status().readiness(), Readiness::Starting);
    wake.wake();
    running.handle().request();
    elapse(SECOND * 3).await;
    assert_eq!(reader.snapshot().invocations, 1);
    assert!(!reader.snapshot().acknowledged);
    assert_eq!(
        reader.snapshot().completion,
        PeriodicCompletion::AbandonedDuringStartup
    );
    release.notify_one();
    running.wait_checked().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_wake_run_deadline_is_retained_and_the_next_wake_has_a_fresh_budget() {
    let mut supervisor = supervisor();
    let mut step = Step::First;
    let (reader, wake) = register_wakeable_in(
        &mut supervisor,
        "storage.refresh",
        WakeablePolicy::new(
            immediate(SECOND * 30, SECOND, PeriodicShutdown::StopAtDrain),
            SECOND * 2,
        )
        .unwrap(),
        move |scope| {
            let step = std::mem::replace(&mut step, Step::Later);
            async move {
                assert_eq!(scope.remaining(), SECOND);
                match step {
                    Step::First => succeeded(),
                    Step::Later => std::future::pending::<Run>().await,
                }
            }
        },
    )
    .unwrap();
    let running = supervisor.start();
    running.status().wait_ready().await.unwrap();
    wake.wake();
    elapse(SECOND * 2).await;
    wake.wake(); // Retained across the active run's deadline.
    elapse(SECOND).await;
    assert_eq!(reader.snapshot().deadline_exceeded, 1);
    elapse(SECOND).await;
    assert_eq!(reader.snapshot().invocations, 3);
    let report = running.shutdown_checked().await.unwrap();
    let summary = &report.report().periodic[0].summary;
    assert_eq!(summary.succeeded, 1);
    assert_eq!(summary.deadline_exceeded, 1);
    assert_eq!(summary.stop_interrupted, 1);
}

#[derive(Clone, Copy)]
enum Terminal {
    Fatal,
    Panic,
}

#[tokio::test(start_paused = true)]
async fn wakes_after_fatal_or_panicked_execution_cannot_restart_the_component() {
    for terminal in [Terminal::Fatal, Terminal::Panic] {
        let mut supervisor = supervisor();
        let (reader, wake) = register_wakeable_in(
            &mut supervisor,
            "storage.refresh",
            WakeablePolicy::new(
                immediate(SECOND * 30, SECOND, PeriodicShutdown::StopAtDrain),
                SECOND,
            )
            .unwrap(),
            move |_| async move {
                match terminal {
                    Terminal::Fatal => escalated(1),
                    Terminal::Panic => panic!("generic maintenance failure"),
                }
            },
        )
        .unwrap();
        let report = supervisor.start().wait().await.unwrap();
        assert!(!report.is_success());
        assert_eq!(
            reader.snapshot().completion,
            match terminal {
                Terminal::Fatal => PeriodicCompletion::Fatal,
                Terminal::Panic => PeriodicCompletion::Pending,
            }
        );
        for _ in 0..100 {
            wake.wake();
        }
        elapse(Duration::from_secs(60)).await;
        assert_eq!(reader.snapshot().invocations, 1);
    }
}
