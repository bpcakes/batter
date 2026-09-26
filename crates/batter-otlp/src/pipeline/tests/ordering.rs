//! Final export ordering against real supervisor, startup and cleanup outcomes.

use super::{super::Schedule, Harness, QUIET, collector::Behavior, counter, find, series};
use crate::diagnostics::{Closure, DiagnosticClosure, ExportFailure, ExportOutcome, FinalCoverage};
use batter_core::{
    cleanup::CleanupBudget,
    lifecycle::{ShutdownBudget, Supervisor},
    startup::Startup,
    telemetry::metrics as catalog,
};
use std::time::Duration;
use tokio::{sync::oneshot, time::Instant};

const CLOSED: DiagnosticClosure = DiagnosticClosure {
    exporter: Closure::Closed,
    provider: Closure::Closed,
};

fn cleanup_budget() -> CleanupBudget {
    let second = Duration::from_secs(1);
    CleanupBudget::new(second * 5, second * 5, second).unwrap()
}

fn supervisor() -> Supervisor {
    let second = Duration::from_secs(1);
    Supervisor::new(ShutdownBudget::new(second, second, second, cleanup_budget()).unwrap())
}

#[test]
fn final_export_follows_cleanup_and_the_shutdown_metric_after_stopped() {
    let harness = Harness::new(Behavior::Accept);
    let (recorder, session) = harness.pipeline(QUIET);
    let collector = harness.collector();
    let (_, report) = harness.run(&recorder, async {
        let mut supervisor = supervisor();
        supervisor
            .register("component", |startup| async move {
                let shutdown = startup.acknowledge_started();
                shutdown.draining().await;
                Ok(shutdown.stopped())
            })
            .unwrap();
        let (entered, cleanup_started) = oneshot::channel();
        let (release, released) = oneshot::channel::<()>();
        supervisor
            .on_cleanup("resource", move || async move {
                let _ = entered.send(());
                let _ = released.await;
                Ok(())
            })
            .unwrap();
        let running = supervisor.start();
        let service = async move { running.shutdown_checked().await.map(|_| ()) };
        let control = async {
            cleanup_started.await.unwrap();
            // Drain, readiness loss and cleanup are all in progress: none of
            // them can begin finalization.
            tokio::time::sleep(Duration::from_millis(100)).await;
            assert!(collector.requests().is_empty());
            release.send(()).unwrap();
        };
        let (output, ()) = tokio::join!(
            session.around(service, |_| FinalCoverage::Reported),
            control
        );
        output
    });
    assert_eq!(report.final_export, ExportOutcome::Acknowledged);
    assert_eq!(report.closure, CLOSED);
    let requests = collector.requests();
    assert_eq!(requests.len(), 1);
    let exported = series(&requests[0]);
    let task = [
        ("kind", "component"),
        ("task", "component"),
        ("outcome", "stopped"),
    ];
    assert_eq!(counter(&exported, catalog::TASK_EXITS, &task), Some(1));
    let hook = [("outcome", "succeeded")];
    assert_eq!(counter(&exported, catalog::CLEANUP_HOOKS, &hook), Some(1));
    let shutdown = [("cause", "requested"), ("result", "success")];
    assert_eq!(counter(&exported, catalog::SHUTDOWNS, &shutdown), Some(1));
    assert!(
        find(
            &exported,
            catalog::SHUTDOWN_DURATION,
            &[("result", "success")]
        )
        .is_some()
    );
}

#[test]
fn startup_failure_cleanup_is_exported_without_a_shutdown() {
    let harness = Harness::new(Behavior::Accept);
    let (recorder, session) = harness.pipeline(QUIET);
    let (_, report) = harness.run(&recorder, async {
        let context = batter_core::operation::OperationOwner::new(Duration::from_secs(5))
            .unwrap()
            .into_context();
        let mut starting = Startup::scoped(supervisor(), context, cleanup_budget(), |scope| {
            Box::pin(async move {
                scope
                    .stage("resource.acquire")
                    .map_err(std::io::Error::other)?;
                scope
                    .reserve_cleanup("resource")
                    .map_err(std::io::Error::other)?
                    .register(|| async { Ok(()) });
                Err(std::io::Error::other("startup failure"))
            })
        })
        .start();
        let service = async move { starting.wait().await.map(|_| ()) };
        session.around(service, |_| FinalCoverage::Reported).await
    });
    assert_eq!(report.final_export, ExportOutcome::Acknowledged);
    let exported = series(&harness.collector().requests()[0]);
    assert_eq!(
        counter(
            &exported,
            catalog::CLEANUP_HOOKS,
            &[("outcome", "succeeded")]
        ),
        Some(1)
    );
    assert!(find(&exported, catalog::SHUTDOWNS, &[]).is_none());
}

#[test]
fn final_allowance_expiry_is_typed_and_resources_still_close_once() {
    let harness = Harness::new(Behavior::StallHeaders);
    let schedule = Schedule {
        final_allowance: Duration::from_millis(300),
        ..QUIET
    };
    let (recorder, session) = harness.pipeline(schedule);
    let started = Instant::now();
    let (_, report) = harness.run(&recorder, async {
        session.around(async {}, |_| FinalCoverage::Reported).await
    });
    assert!(started.elapsed() < schedule.final_allowance + Duration::from_secs(1));
    assert_eq!(
        report.final_export,
        ExportOutcome::Failed(ExportFailure::HeadersTimedOut)
    );
    assert_eq!(report.closure, CLOSED);
    assert_eq!(harness.collector().requests().len(), 1);
}

#[test]
fn in_flight_periodic_export_settles_before_the_final_snapshot() {
    let harness = Harness::new(Behavior::StallHeaders);
    let schedule = Schedule {
        interval: Duration::from_millis(50),
        attempt: Duration::from_millis(200),
        final_allowance: Duration::from_secs(2),
    };
    let (recorder, session) = harness.pipeline(schedule);
    let collector = harness.collector();
    let (_, report) = harness.run(&recorder, async {
        let service = async {
            // Ends while the second periodic attempt is stalled in flight.
            tokio::time::sleep(Duration::from_millis(350)).await;
            collector.set(Behavior::Accept);
        };
        session.around(service, |_| FinalCoverage::Reported).await
    });
    let periodic = report.periodic;
    assert!(periodic.attempts >= 2, "{periodic:?}");
    assert_eq!(periodic.failed, periodic.attempts);
    assert_eq!(periodic.first_failure, Some(ExportFailure::HeadersTimedOut));
    assert_eq!(periodic.last_failure, Some(ExportFailure::HeadersTimedOut));
    assert!(periodic.coalesced_intervals >= 1, "{periodic:?}");
    assert_eq!(report.final_export, ExportOutcome::Acknowledged);
    // Every periodic attempt settled once, then exactly one final request.
    assert_eq!(
        collector.requests().len() as u64,
        periodic.attempts + 1,
        "{periodic:?}"
    );
}

#[test]
fn tiny_intervals_coalesce_without_iterating_over_elapsed_ticks() {
    let harness = Harness::new(Behavior::StallHeaders);
    let schedule = Schedule {
        interval: Duration::from_nanos(1),
        attempt: Duration::from_millis(100),
        final_allowance: Duration::from_millis(100),
    };
    let (recorder, session) = harness.pipeline(schedule);
    let started = Instant::now();
    let (_, report) = harness.run(&recorder, async {
        session
            .around(tokio::time::sleep(Duration::from_millis(20)), |_| {
                FinalCoverage::Reported
            })
            .await
    });
    assert!(started.elapsed() < Duration::from_secs(1));
    assert!(report.periodic.coalesced_intervals > 1_000_000);
    assert_eq!(report.periodic.attempts, 1);
    assert_eq!(report.closure, CLOSED);
}
