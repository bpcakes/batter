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
    install(capture.clone()).expect("first installation");
    let described = capture.described();
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
    let result = context
        .run("example.read", |_| async { Ok::<_, Infallible>(()) })
        .await;
    assert!(result.is_ok());
    assert_eq!(
        capture.count(OPERATION_COMPLETIONS, &[("operation", "example.read")]),
        1.0
    );
    // A second installation is rejected and hands the recorder back.
    assert!(install(Capture::unbounded()).is_err());
}
