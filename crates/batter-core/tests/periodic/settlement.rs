//! Support stopping against native settlement, clocks and dependency cleanup.

use super::support::*;
use batter_core::{
    cleanup::SkipReason,
    lifecycle::ManagedComponent,
    operation::OperationOwner,
    periodic::{PeriodicCompletion, PeriodicShutdown, register_periodic_in},
};
use std::{future::pending, time::Duration};
use tokio::{
    sync::{mpsc, oneshot},
    time::{Instant, sleep},
};

fn native_context() -> batter_core::operation::OperationContext {
    OperationOwner::new(SECOND * 10).unwrap().into_context()
}

/// Register a native component whose settlement finishes `after` its stop was
/// requested, reporting the selected cooperation.
fn register_native(
    supervisor: &mut batter_core::lifecycle::Supervisor,
    after: Duration,
    joined: bool,
) {
    let (stopped, requested) = oneshot::channel::<()>();
    let mut stopped = Some(stopped);
    supervisor
        .register_managed("native", native_context(), move |_| {
            Ok(ManagedComponent::new(
                async { Ok(()) },
                pending(),
                move |started| {
                    if let Some(stopped) = stopped.take() {
                        let _ = stopped.send(());
                    }
                    started
                },
                async move {
                    let _ = requested.await;
                    sleep(after).await;
                    NativeReport { joined }
                },
            ))
        })
        .unwrap();
}

fn register_support(supervisor: &mut batter_core::lifecycle::Supervisor, schedule: &Schedule) {
    let recorded = schedule.clone();
    let origin = Instant::now();
    register_periodic_in(
        supervisor,
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
}

#[tokio::test(start_paused = true)]
async fn pending_native_settlement_does_not_stop_support_early() {
    let schedule = Schedule::default();
    let mut supervisor = supervisor_with(SECOND * 10);
    register_native(&mut supervisor, Duration::from_millis(2_500), true);
    register_support(&mut supervisor, &schedule);
    let running = supervisor.start();
    running.status().wait_ready().await.unwrap();
    running.handle().request();
    let success = running.wait_checked().await.unwrap();
    // Support stops only once the retained native evidence permits cleanup.
    assert_eq!(schedule.starts(), vec![Duration::ZERO, SECOND, SECOND * 2]);
    let report = success.report();
    assert!(report.managed[0].outcome.allows_dependency_cleanup());
    assert_eq!(
        report.periodic[0].summary.completion,
        PeriodicCompletion::Stopped
    );
    assert!(report.cleanup.skipped.is_empty());
}

#[tokio::test(start_paused = true)]
async fn unsafe_native_settlement_keeps_support_until_forced_cancellation() {
    let schedule = Schedule::default();
    let mut supervisor = supervisor_with(Duration::from_millis(2_500));
    // Settlement publishes at once, but it does not permit dependency cleanup,
    // so a joined wrapper and a finished report are still not a stopped proof.
    register_native(&mut supervisor, Duration::ZERO, false);
    register_support(&mut supervisor, &schedule);
    supervisor
        .on_cleanup("dependency", || async {
            panic!("uncooperative native work must block cleanup")
        })
        .unwrap();
    let running = supervisor.start();
    running.status().wait_ready().await.unwrap();
    let requested = Instant::now();
    running.handle().request();
    let report = running.wait_report().await.unwrap();
    assert_eq!(schedule.starts(), vec![Duration::ZERO, SECOND, SECOND * 2]);
    assert!(!report.is_success());
    assert!(!report.managed[0].outcome.allows_dependency_cleanup());
    assert_eq!(report.cleanup.skipped[0].reason, SkipReason::UnsafeTaskExit);
    // The support loop ended at the existing forced-cancellation boundary.
    assert!(Instant::now() >= requested + Duration::from_millis(2_500));
    assert_eq!(
        report.periodic[0].summary.completion,
        PeriodicCompletion::Stopped
    );
    drop(report);
}

#[tokio::test(start_paused = true)]
async fn support_run_admitted_during_drain_receives_the_forced_deadline() {
    let drain = Duration::from_millis(2_500);
    let budget = SECOND * 10;
    let mut supervisor = supervisor_with(drain);
    // Uncooperative settlement keeps support admission open until force-cancel.
    register_native(&mut supervisor, Duration::ZERO, false);
    let (observed, mut deadlines) = mpsc::unbounded_channel();
    register_periodic_in(
        &mut supervisor,
        "lease.renewal",
        immediate(SECOND, budget, PeriodicShutdown::SupportThroughDrain),
        move |context| {
            observed.send((Instant::now(), context.deadline())).unwrap();
            async { succeeded() }
        },
    )
    .unwrap();
    let running = supervisor.start();
    running.status().wait_ready().await.unwrap();
    let (started, deadline) = deadlines.recv().await.unwrap();
    assert_eq!(deadline, started + budget);

    let requested = Instant::now();
    running.handle().request();
    let (started, deadline) = deadlines.recv().await.unwrap();
    let forced_at = requested + drain;
    assert_eq!(started, requested + SECOND);
    assert!(started + budget > forced_at);
    // Inspect the callback's deadline directly: global token cancellation would
    // otherwise hide a missing cap if the test only observed termination time.
    assert_eq!(deadline, forced_at);
    let report = running.wait_report().await.unwrap();
    assert!(!report.managed[0].outcome.allows_dependency_cleanup());
}

#[tokio::test(start_paused = true)]
async fn an_earlier_native_stop_clock_tightens_support_scheduling() {
    let schedule = Schedule::default();
    let origin = Instant::now();
    let mut supervisor = supervisor_with(Duration::from_millis(3_500));
    supervisor
        .register_managed("native", native_context(), move |_| {
            Ok(ManagedComponent::new(
                async { Ok(()) },
                pending(),
                // The native layer reveals that it stopped before this drain.
                move |parent_started| origin.min(parent_started),
                pending::<NativeReport>(),
            ))
        })
        .unwrap();
    register_support(&mut supervisor, &schedule);
    let running = supervisor.start();
    running.status().wait_ready().await.unwrap();
    sleep(SECOND * 2).await;
    running.handle().request();
    let report = running.wait_report().await.unwrap();
    assert!(!report.is_success());
    // Forced cancellation ran from the tightened clock at the origin plus the
    // drain allowance, not from the later locally observed drain.
    assert_eq!(
        schedule.starts(),
        vec![Duration::ZERO, SECOND, SECOND * 2, SECOND * 3],
    );
    drop(report);
}

#[tokio::test(start_paused = true)]
async fn support_work_is_destroyed_before_dependency_cleanup() {
    let order = Order::default();
    let schedule = Schedule::default();
    let mut supervisor = supervisor_with(SECOND * 10);
    supervisor
        .register("worker", |startup| async move {
            let running = startup.acknowledge_started();
            running.draining().await;
            sleep(Duration::from_millis(1_500)).await;
            Ok(running.stopped())
        })
        .unwrap();
    let recorded = schedule.clone();
    let origin = Instant::now();
    let witness = Destroyed(order.clone(), "support.destroyed");
    register_periodic_in(
        &mut supervisor,
        "lease.renewal",
        immediate(SECOND, SECOND, PeriodicShutdown::SupportThroughDrain),
        move |_| {
            // The witness belongs to the closure itself, so it is destroyed
            // exactly once, when the whole runner future is destroyed.
            let _ = &witness;
            let started = recorded.started(origin);
            async move {
                drop(started);
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
    drop(running.shutdown_checked().await.unwrap());
    assert_eq!(order.observed(), vec!["support.destroyed", "cleanup"]);
    assert_eq!(schedule.starts(), vec![Duration::ZERO, SECOND]);
}
