//! Report formatting never discloses retained task or cleanup error contents.
//!
//! `ShutdownReport`, `TaskRecord`, `CleanupReport` and `CleanupRecord` redact
//! Debug and Display like the command and startup reports. Retained errors stay
//! reachable only through the public fields, so disclosure is an explicit read.

use batter_core::{
    BoxError,
    cleanup::{CleanupBudget, CleanupOutcome},
    lifecycle::{ComponentExit, ShutdownBudget, ShutdownFailure, Supervisor, TaskOutcome},
};
use std::{error::Error, time::Duration};

fn budget() -> ShutdownBudget {
    let second = Duration::from_secs(1);
    let cleanup = CleanupBudget::new(second, second, second).unwrap();
    ShutdownBudget::new(second, second, second, cleanup).unwrap()
}

#[tokio::test]
async fn shutdown_and_cleanup_reports_redact_error_contents_in_debug_and_display() {
    let mut supervisor = Supervisor::new(budget());
    supervisor
        .register("component", |startup| async move {
            let _running = startup.acknowledge_started();
            Err::<ComponentExit, BoxError>(std::io::Error::other("task-secret-marker").into())
        })
        .unwrap();
    supervisor
        .on_cleanup("dependency", || async {
            Err(std::io::Error::other("cleanup-secret-marker").into())
        })
        .unwrap();

    let running = supervisor.start();
    let report = running.wait_report().await.unwrap();
    assert!(!report.is_success());
    assert_eq!(report.tasks[0].outcome, TaskOutcome::Failed);
    assert_eq!(report.cleanup.records[0].outcome, CleanupOutcome::Failed);

    let formatted = format!(
        "{report:?} {report} {:?} {} {:?} {:?}",
        *report, *report, report.tasks[0], report.cleanup
    );
    assert!(!formatted.contains("secret-marker"), "{formatted}");
    assert!(
        formatted.contains("error: Some(\"retained\")"),
        "{formatted}"
    );

    let failure = running.wait_checked().await.unwrap_err();
    let ShutdownFailure::Report(checked) = &failure else {
        panic!("failed report must remain accessible")
    };
    assert!(std::ptr::eq(&*report, &**checked));
    let wrapper = format!("{failure:?} {failure}");
    assert!(!wrapper.contains("secret-marker"), "{wrapper}");
    assert_eq!(
        failure.source().unwrap().to_string(),
        format!("{}", *report)
    );

    // Disclosure is a deliberate field read, never a formatting side effect.
    assert_eq!(
        report.tasks[0].error.as_ref().unwrap().to_string(),
        "task-secret-marker"
    );
    assert_eq!(
        report.cleanup.records[0]
            .error
            .as_ref()
            .unwrap()
            .to_string(),
        "cleanup-secret-marker"
    );
}
