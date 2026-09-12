use super::*;
use batter::{
    cleanup::CleanupOutcome,
    lifecycle::{ShutdownCause, TaskOutcome},
};

const STARTUP_DETAIL: &str = "sensitive startup detail";
const CLEANUP_DETAIL: &str = "sensitive cleanup detail";
const SHUTDOWN_DETAIL: &str = "sensitive shutdown detail";
const SHUTDOWN_CLEANUP_DETAIL: &str = "sensitive shutdown cleanup detail";

fn assert_process_error_is_redacted(error: &ProcessFailure, details: &[&str]) {
    assert_eq!(error.to_string(), "process failed");
    let debug = format!("{error:?}");
    assert_eq!(debug, "process failed");
    for detail in details {
        assert!(!debug.contains(detail));
    }
}

#[test]
fn process_boundary_retains_and_redacts_an_early_failure() {
    let result: Result<(), BoxError> = Err(std::io::Error::other(STARTUP_DETAIL).into());
    let error = process_result(result).expect_err("early failure must reach the boundary");
    let source = std::error::Error::source(&error)
        .expect("the concrete early failure must remain the source");

    assert!(std::ptr::addr_eq(source, error.cause.as_ref()));
    assert_eq!(source.to_string(), STARTUP_DETAIL);
    assert_process_error_is_redacted(&error, &[STARTUP_DETAIL]);
}

#[tokio::test]
async fn startup_failure_path_retains_real_cleanup_diagnostics() {
    let mut supervisor = Supervisor::new(support::shutdown_budget());
    supervisor
        .on_cleanup("dependency", || async { Ok(()) })
        .unwrap();
    supervisor
        .on_cleanup("resource", || async {
            Err(std::io::Error::other(CLEANUP_DETAIL).into())
        })
        .unwrap();

    let result = complete_startup(
        supervisor,
        Err(std::io::Error::other(STARTUP_DETAIL).into()),
    )
    .await
    .map(|_| ());
    let error = process_result(result).expect_err("startup failure must reach the boundary");
    let failure = startup_failure(&error.cause);
    let source = std::error::Error::source(failure).unwrap();
    let initialization = source
        .downcast_ref::<InitializationError<ProcessFailure>>()
        .unwrap();
    let wrapped = std::error::Error::source(initialization)
        .unwrap()
        .downcast_ref::<ProcessFailure>()
        .unwrap();
    assert!(std::ptr::addr_eq(
        wrapped.cause.as_ref(),
        application_cause(failure).as_ref()
    ));
    assert_eq!(wrapped.cause.to_string(), STARTUP_DETAIL);
    assert_eq!(failure.cleanup.records.len(), 2);
    assert_eq!(failure.cleanup.records[0].name, "resource");
    assert_eq!(failure.cleanup.records[0].outcome, CleanupOutcome::Failed);
    assert_eq!(
        failure.cleanup.records[0]
            .error
            .as_deref()
            .expect("cleanup error must be retained")
            .to_string(),
        CLEANUP_DETAIL
    );
    assert_eq!(failure.cleanup.records[1].name, "dependency");
    assert_eq!(
        failure.cleanup.records[1].outcome,
        CleanupOutcome::Succeeded
    );
    assert!(failure.cleanup.skipped.is_empty());

    let diagnostic = format!("{failure:?}");
    assert_eq!(
        diagnostic,
        "startup failed at startup; cleanup: 1 unsuccessful, 0 skipped"
    );
    assert!(!diagnostic.contains(STARTUP_DETAIL));
    assert!(!diagnostic.contains(CLEANUP_DETAIL));
    assert_process_error_is_redacted(&error, &[STARTUP_DETAIL, CLEANUP_DETAIL]);
}

#[tokio::test]
async fn shutdown_failure_path_retains_the_complete_report() {
    let mut supervisor = Supervisor::new(support::shutdown_budget());
    supervisor
        .on_cleanup("resource", || async {
            Err(std::io::Error::other(SHUTDOWN_CLEANUP_DETAIL).into())
        })
        .unwrap();
    supervisor
        .register("component", |shutdown| async move {
            shutdown.mark_started();
            Err(std::io::Error::other(SHUTDOWN_DETAIL).into())
        })
        .unwrap();

    let running = supervisor.start();
    let report = running
        .wait()
        .await
        .expect("component failure must still publish a shutdown report");
    let error =
        process_result(check_shutdown(Ok(report)).map_err(|error| Box::new(error) as BoxError))
            .expect_err("unsuccessful shutdown must reach the process boundary");
    let failure = std::error::Error::source(&error)
        .expect("typed shutdown failure must remain the process source")
        .downcast_ref::<ShutdownFailure>()
        .expect("typed shutdown failure must survive BoxError conversion");
    let report_source = std::error::Error::source(failure)
        .expect("shutdown report must remain in the error source chain");

    let report = shutdown_report(failure);
    assert!(std::ptr::addr_eq(report_source, &**report));
    assert_eq!(report.cause, ShutdownCause::ComponentExit("component"));
    assert_eq!(report.tasks.len(), 1);
    assert_eq!(report.tasks[0].outcome, TaskOutcome::Failed);
    assert_eq!(
        report.tasks[0]
            .error
            .as_deref()
            .expect("task error must be retained")
            .to_string(),
        SHUTDOWN_DETAIL
    );
    assert_eq!(report.cleanup.records.len(), 1);
    assert_eq!(
        report.cleanup.records[0]
            .error
            .as_deref()
            .expect("shutdown cleanup error must be retained")
            .to_string(),
        SHUTDOWN_CLEANUP_DETAIL
    );

    let diagnostic = format!("{failure:?}");
    assert!(!diagnostic.contains(SHUTDOWN_DETAIL));
    assert!(!diagnostic.contains(SHUTDOWN_CLEANUP_DETAIL));
    assert_process_error_is_redacted(&error, &[SHUTDOWN_DETAIL, SHUTDOWN_CLEANUP_DETAIL]);
}
