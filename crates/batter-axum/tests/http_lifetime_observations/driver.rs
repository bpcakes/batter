//! One owner for startup, joined exercise and independently awaited teardown.
use super::{cases, fixture::Fixture, resource::DropEvent};
use crate::{
    capture::Capture,
    http_process::{EXERCISE, STARTUP, TEARDOWN},
};
use batter::lifecycle::SharedShutdownReport;
use std::sync::Arc;
use tokio::time::{Instant, timeout, timeout_at};

pub async fn run(case: &str, capture: Arc<Capture>) {
    let startup_deadline = Instant::now() + STARTUP;
    let (fixture, running) = timeout_at(startup_deadline, Fixture::start(case))
        .await
        .expect("fixture construction deadline; no running owner returned");
    let startup = timeout_at(startup_deadline, fixture.handle.wait_ready()).await;
    let ready = matches!(startup, Ok(Ok(())));
    let exercise = if ready {
        let observed = fixture.clone();
        let name = case.to_owned();
        let capture = capture.clone();
        Some(
            tokio::spawn(async move {
                // Timeout owns the exercise future, not its JoinHandle. Awaiting
                // the task observes future destruction before releasing teardown.
                timeout(EXERCISE, cases::exercise(&name, &observed, &capture)).await
            })
            .await,
        )
    } else {
        None
    };

    if matches!(exercise, Some(Ok(Ok(())))) {
        fixture.events.record("exercise-complete");
    }
    fixture.release_handler.notify_one();
    fixture.release_body.notify_one();
    fixture.release_graceful.notify_one();
    fixture.handle.request();
    let teardown_deadline = Instant::now() + TEARDOWN;
    // Keep both the running owner and any report outside reconciliation. Later
    // body/socket assertion failure or timeout must not erase an observed report.
    let report = timeout_at(teardown_deadline, running.wait()).await;
    let reconciliation = if let Ok(Ok(report)) = &report {
        fixture.events.record("terminal-report");
        let report = report.clone();
        let observed = fixture.clone();
        let name = case.to_owned();
        Some(
            tokio::spawn(async move {
                timeout_at(
                    teardown_deadline,
                    reconcile(&name, &observed, &report, ready),
                )
                .await
            })
            .await,
        )
    } else {
        None
    };
    assert!(
        ready
            && matches!(exercise, Some(Ok(Ok(()))))
            && matches!(report, Ok(Ok(_)))
            && matches!(reconciliation, Some(Ok(Ok(())))),
        "startup={startup:?}; exercise={exercise:?}; report={report:?}; reconciliation={reconciliation:?}; events: {:?}; interrupted waits: {:?}; trace: {}",
        fixture.events.snapshot(),
        fixture.events.diagnostics(),
        capture.text(),
    );
}

async fn reconcile(case: &str, fixture: &Fixture, report: &SharedShutdownReport, ready: bool) {
    if !ready {
        // No accepted connection exists when startup never acknowledged readiness.
        assert!(report.is_success(), "{report:?}");
        assert_eq!(fixture.events.count("finalizer"), 1);
        return;
    }
    cases::check_shutdown(case, fixture, report);
    if matches!(
        case,
        "missing_reconciliation_event" | "delayed_missing_reconciliation_event"
    ) {
        let _drop = DropEvent(fixture.events.clone(), "reconciliation-resource-drop");
        fixture.events.wait("never-reconciled").await;
    }
    if case == "slow_failure" {
        let _drop = DropEvent(fixture.events.clone(), "teardown-resource-drop");
        // Yielding failure after a real report, like a missing body-drop event.
        std::future::pending::<()>().await;
    }
    if fixture.events.count("body-pending") > 0 {
        fixture.events.wait("body-drop").await;
    }
}
