//! The two library-owned stopping classes and their drain ordering.

use super::support::*;
use batter_core::{
    lifecycle::{Fatal, ProcessAdmissionError, ProcessCapacity, ShutdownCause, ShutdownFailure},
    periodic::{PeriodicCompletion, PeriodicShutdown, register_periodic_in},
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{
    sync::Notify,
    time::{Instant, sleep},
};

#[tokio::test(start_paused = true)]
async fn stop_at_drain_admits_no_run_after_the_stop_transition() {
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
    sleep(Duration::from_millis(2_500)).await;
    let before = schedule.count();
    assert_eq!(before, 3);
    // The stop transition is synchronous, so nothing later is admitted.
    running.handle().request();
    let success = running.wait_checked().await.unwrap();
    assert_eq!(schedule.count(), before);
    let summary = &success.report().periodic[0].summary;
    assert_eq!(summary.invocations, 3);
    assert_eq!(summary.completion, PeriodicCompletion::Stopped);
}

#[tokio::test(start_paused = true)]
async fn stop_at_drain_destroys_an_active_run_before_dependency_cleanup() {
    let order = Order::default();
    let mut supervisor = supervisor();
    let entered = Arc::new(Notify::new());
    let signalled = entered.clone();
    let observed = order.clone();
    register_periodic_in(
        &mut supervisor,
        "storage.pruning",
        immediate(SECOND, SECOND * 100, PeriodicShutdown::StopAtDrain),
        move |_| {
            let signalled = signalled.clone();
            let witness = Destroyed(observed.clone(), "run.destroyed");
            async move {
                signalled.notify_one();
                std::future::pending::<()>().await;
                drop(witness);
                succeeded()
            }
        },
    )
    .unwrap();
    let closing = order.clone();
    supervisor
        .on_cleanup("dependency", move || {
            let closing = closing.clone();
            async move {
                closing.push("cleanup");
                Ok(())
            }
        })
        .unwrap();
    let running = supervisor.start();
    running.status().wait_ready().await.unwrap();
    entered.notified().await;
    let success = running.shutdown_checked().await.unwrap();
    assert_eq!(order.observed(), vec!["run.destroyed", "cleanup"]);
    assert_eq!(success.report().periodic[0].summary.stop_interrupted, 1);
}

#[tokio::test(start_paused = true)]
async fn support_continues_while_an_ordinary_component_drains() {
    let schedule = Schedule::default();
    let mut supervisor = supervisor_with(SECOND * 10);
    supervisor
        .register("worker", |startup| async move {
            let running = startup.acknowledge_started();
            running.draining().await;
            // Ordinary work keeps using its dependencies while it finishes.
            sleep(Duration::from_millis(2_500)).await;
            Ok(running.stopped())
        })
        .unwrap();
    let recorded = schedule.clone();
    let origin = Instant::now();
    register_periodic_in(
        &mut supervisor,
        "lease.renewal",
        immediate(SECOND, SECOND, PeriodicShutdown::SupportThroughDrain),
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
    running.handle().request();
    let success = running.wait_checked().await.unwrap();
    // Support ran through the ordinary component's whole drain, then stopped
    // as soon as that component had been joined.
    assert_eq!(schedule.starts(), vec![Duration::ZERO, SECOND, SECOND * 2]);
    let summary = &success.report().periodic[0].summary;
    assert_eq!(summary.invocations, 3);
    assert_eq!(summary.completion, PeriodicCompletion::Stopped);
}

#[tokio::test(start_paused = true)]
async fn support_continues_while_admitted_finite_descendants_drain() {
    let schedule = Schedule::default();
    let mut supervisor = supervisor_process(ProcessCapacity::new(4).unwrap());
    let recorded = schedule.clone();
    let origin = Instant::now();
    register_periodic_in(
        &mut supervisor,
        "lease.renewal",
        immediate(SECOND, SECOND, PeriodicShutdown::SupportThroughDrain),
        move |_| {
            let started = recorded.started(origin);
            async move {
                drop(started);
                succeeded()
            }
        },
    )
    .unwrap();
    let process = supervisor.process_handle().unwrap();
    let running = supervisor.start();
    running.status().wait_ready().await.unwrap();
    let receipt = process
        .try_spawn("refresh", |scope| async move {
            scope.signal().draining().await;
            // Only an active process scope can admit a descendant during drain.
            let child = scope
                .try_spawn("refresh.child", |_| async {
                    sleep(Duration::from_millis(2_500)).await;
                    Ok::<_, Fatal<std::io::Error>>(())
                })
                .expect("an active ancestor admits its descendant");
            drop(child);
            Ok::<_, Fatal<std::io::Error>>(())
        })
        .unwrap();
    running.handle().request();
    // Root admission is closed even though support work continues.
    assert!(matches!(
        process.try_spawn("rejected", |_| async { Ok::<_, Fatal<std::io::Error>>(()) }),
        Err(ProcessAdmissionError::Closed),
    ));
    drop(receipt);
    let success = running.wait_checked().await.unwrap();
    assert_eq!(schedule.starts(), vec![Duration::ZERO, SECOND, SECOND * 2]);
    assert_eq!(success.report().completed_process_tasks, 2);
}

fn supervisor_process(capacity: ProcessCapacity) -> batter_core::lifecycle::Supervisor {
    use batter_core::{cleanup::CleanupBudget, lifecycle::ShutdownBudget};
    batter_core::lifecycle::Supervisor::with_process_capacity(
        ShutdownBudget::new(
            SECOND * 10,
            SECOND,
            SECOND,
            CleanupBudget::new(SECOND, SECOND, SECOND).unwrap(),
        )
        .unwrap(),
        capacity,
    )
}

#[tokio::test(start_paused = true)]
async fn an_ordinary_failure_closes_ordinary_admission_but_not_support() {
    let schedule = Schedule::default();
    let mut supervisor = supervisor_with(SECOND * 10);
    let fail = Arc::new(Notify::new());
    let failing = fail.clone();
    supervisor
        .register("worker.a", move |startup| async move {
            let running = startup.acknowledge_started();
            failing.notified().await;
            drop(running);
            Err(Box::new(Attempt(0)) as batter_core::BoxError)
        })
        .unwrap();
    supervisor
        .register("worker.b", |startup| async move {
            let running = startup.acknowledge_started();
            running.draining().await;
            sleep(Duration::from_millis(2_500)).await;
            Ok(running.stopped())
        })
        .unwrap();
    let recorded = schedule.clone();
    let origin = Instant::now();
    register_periodic_in(
        &mut supervisor,
        "lease.renewal",
        immediate(SECOND, SECOND, PeriodicShutdown::SupportThroughDrain),
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
    fail.notify_one();
    let ShutdownFailure::Report(report) = running.wait_checked().await.unwrap_err() else {
        panic!("expected the failed component report")
    };
    assert_eq!(report.cause, ShutdownCause::ComponentExit("worker.a"));
    assert_eq!(schedule.starts(), vec![Duration::ZERO, SECOND, SECOND * 2]);
    assert_eq!(
        report.periodic[0].summary.completion,
        PeriodicCompletion::Stopped
    );
}

#[tokio::test(start_paused = true)]
async fn a_support_only_supervisor_stops_without_consuming_the_grace_budget() {
    let mut supervisor = supervisor_with(SECOND * 30);
    let runs = Arc::new(AtomicUsize::new(0));
    let counted = runs.clone();
    register_periodic_in(
        &mut supervisor,
        "lease.renewal",
        immediate(SECOND, SECOND, PeriodicShutdown::SupportThroughDrain),
        move |_| {
            let counted = counted.clone();
            async move {
                counted.fetch_add(1, Ordering::SeqCst);
                succeeded()
            }
        },
    )
    .unwrap();
    let running = supervisor.start();
    running.status().wait_ready().await.unwrap();
    let requested = Instant::now();
    drop(running.shutdown_checked().await.unwrap());
    // There is no ordinary work, so support closes promptly on drain.
    assert_eq!(Instant::now(), requested);
    assert_eq!(runs.load(Ordering::SeqCst), 1);
}

#[tokio::test(start_paused = true)]
async fn an_escalating_support_run_is_a_retained_component_failure() {
    let mut supervisor = supervisor_with(SECOND * 10);
    let reader = register_periodic_in(
        &mut supervisor,
        "lease.renewal",
        immediate(SECOND, SECOND, PeriodicShutdown::SupportThroughDrain),
        |_| async { escalated(7) },
    )
    .unwrap();
    let running = supervisor.start();
    let ShutdownFailure::Report(report) = running.wait_checked().await.unwrap_err() else {
        panic!("expected the escalated support failure")
    };
    assert_eq!(report.cause, ShutdownCause::ComponentExit("lease.renewal"));
    assert_eq!(
        report.tasks[0]
            .error
            .as_ref()
            .and_then(|error| error.downcast_ref::<Attempt>())
            .map(|attempt| attempt.0),
        Some(7),
    );
    assert_eq!(reader.snapshot().completion, PeriodicCompletion::Fatal);
}
