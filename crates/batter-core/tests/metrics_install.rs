//! `install` owns the process-wide recorder; this binary owns that global.
#![cfg(feature = "metrics")]

#[path = "metrics/capture.rs"]
mod capture;

use batter_core::{operation::OperationOwner, telemetry::metrics::*};
use capture::Capture;
use std::{convert::Infallible, time::Duration};

#[tokio::test]
async fn install_publishes_the_catalog_to_the_recorder_it_installs() {
    let capture = Capture::unbounded();
    let local = Capture::unbounded();
    let local_guard = facade::set_default_local_recorder(&local);
    install(capture.clone()).expect("first installation");
    assert!(local.described().is_empty());
    let described = capture.described();
    assert_eq!(described.len(), 10);
    for name in [
        OPERATION_COMPLETIONS,
        OPERATION_DURATION,
        RETRY_ATTEMPTS,
        RETRY_EXECUTIONS,
        ADMISSION_DECISIONS,
        TASK_EXITS,
        CLEANUP_HOOKS,
        SHUTDOWNS,
        SHUTDOWN_DURATION,
        LABELS_COALESCED,
    ] {
        assert!(
            described.iter().any(|described| described == name),
            "{name}"
        );
    }
    // The installed recorder receives ordinary recording.
    let context = OperationOwner::new(Duration::from_secs(1))
        .unwrap()
        .into_context();
    context
        .run("example.local", |_| async { Ok::<_, Infallible>(()) })
        .await
        .unwrap();
    assert_eq!(
        local.count(OPERATION_COMPLETIONS, &[("operation", "example.local")]),
        1.0
    );
    assert_eq!(
        capture.count(OPERATION_COMPLETIONS, &[("operation", "example.local")]),
        0.0
    );
    drop(local_guard);
    let result = context
        .run("example.read", |_| async { Ok::<_, Infallible>(()) })
        .await;
    assert!(result.is_ok());
    assert_eq!(
        capture.count(OPERATION_COMPLETIONS, &[("operation", "example.read")]),
        1.0
    );
    // A second installation is rejected and hands the recorder back.
    let rejected = Capture::unbounded();
    let returned = install(rejected.clone()).unwrap_err().into_inner();
    assert!(rejected.described().is_empty());
    facade::with_local_recorder(&returned, || {
        facade::counter!(CLEANUP_HOOKS, "outcome" => "succeeded").increment(1);
    });
    assert_eq!(
        rejected.count(CLEANUP_HOOKS, &[("outcome", "succeeded")]),
        1.0
    );
    assert_eq!(capture.described().len(), 10);
}
