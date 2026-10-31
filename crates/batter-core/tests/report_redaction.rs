//! Report formatting never discloses retained task or cleanup error contents.
//!
//! `ShutdownReport`, `TaskRecord`, `CleanupReport` and `CleanupRecord` redact
//! Debug and Display like the command and startup reports. Retained errors stay
//! reachable only through the public fields, so disclosure is an explicit read.
//! `BlockingError` likewise never formats a retained panic payload.

use batter_core::{
    BoxError,
    admission::{Admission, AdmissionError, BlockingError, Bulkhead, BulkheadCapacity},
    cleanup::{CleanupBudget, CleanupOutcome},
    lifecycle::{ComponentExit, ShutdownBudget, ShutdownFailure, Supervisor, TaskOutcome},
    operation::{Interruption, OperationOwner},
};
use std::{collections::BTreeSet, error::Error, time::Duration};

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

fn boxable<E: Error + Send + Sync + 'static>(_: &E) {}

#[tokio::test]
async fn blocking_error_formatting_never_discloses_panic_payloads() {
    let bulkhead = Bulkhead::new(BulkheadCapacity::new(1).unwrap());
    let context = OperationOwner::new(Duration::from_secs(5))
        .unwrap()
        .into_context();
    let panicked = bulkhead
        .run_blocking(&context, Admission::Reject, || -> u8 {
            panic!("payload-secret-marker")
        })
        .await
        .unwrap_err();
    let BlockingError::Panicked(payload) = &panicked else {
        panic!("a closure panic must be reported as Panicked: {panicked:?}")
    };
    assert_eq!(format!("{panicked:?}"), "Panicked(..)");
    assert_eq!(format!("{panicked:#?}"), "Panicked(..)");
    assert_eq!(panicked.to_string(), "blocking work panicked");
    // Disclosure is a deliberate payload read, never a formatting side effect.
    assert_eq!(
        payload.try_inspect(|value| value.downcast_ref::<&str>().copied()),
        Ok(Some("payload-secret-marker"))
    );
    assert!(panicked.source().is_none());

    let not_admitted =
        BlockingError::NotAdmitted(AdmissionError::Interrupted(Interruption::Cancelled));
    let not_started = BlockingError::NotStarted;
    let abandoned = BlockingError::Abandoned(Interruption::Cancelled);
    assert_eq!(
        format!("{not_admitted:?}"),
        "NotAdmitted(Interrupted(Cancelled))"
    );
    assert_eq!(format!("{not_started:?}"), "NotStarted");
    assert_eq!(format!("{abandoned:?}"), "Abandoned(Cancelled)");
    // Each outcome reads differently, so a log line cannot merge them.
    let displays: BTreeSet<String> = [&not_admitted, &not_started, &abandoned, &panicked]
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(displays.len(), 4, "{displays:?}");

    // Display omits causes; `source()` chains them (Panicked and NotStarted
    // have none), and the variant fields stay readable by matching.
    let overloaded = BlockingError::NotAdmitted(AdmissionError::Overloaded);
    assert_eq!(
        overloaded.source().unwrap().to_string(),
        "concurrency capacity exhausted"
    );
    let expired = BlockingError::Abandoned(Interruption::DeadlineExceeded);
    assert_eq!(
        expired.source().unwrap().downcast_ref::<Interruption>(),
        Some(&Interruption::DeadlineExceeded)
    );
    assert!(not_started.source().is_none());

    // The error boxes like any application error and stays redacted.
    boxable(&panicked);
    let boxed: BoxError = panicked.into();
    let formatted = format!("{boxed:?} {boxed:#?} {boxed}");
    assert!(!formatted.contains("secret-marker"), "{formatted}");
}
