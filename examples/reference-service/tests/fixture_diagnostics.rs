#[path = "support/fixture_diagnostics.rs"]
mod diagnostics;

use batter_sqlx::test_support::FixtureReport;
use batter_sqlx::test_support::{AcquisitionFailure, BodyFailure, DatabaseCleanup, FixtureError};
use diagnostics::{ProbeError, assert_probe};

fn panic_message(result: Result<(), Box<dyn std::error::Error + Send + Sync>>) -> String {
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| assert_probe(result)))
        .expect_err("the test boundary must fail");
    *panic.downcast::<String>().expect("formatted panic")
}

#[test]
fn failed_probe_prints_branches_and_report_counts_without_native_contents() {
    let report = FixtureReport::<(), _> {
        body: Err(BodyFailure::Returned(ProbeError::new("body-secret".into()))),
        acquisitions: vec![Err(AcquisitionFailure::Native(std::sync::Arc::new(
            postgres_test_harness::Error::InvalidConfiguration {
                reason: "producer-secret",
            },
        )))],
        databases: vec![DatabaseCleanup {
            pool_failures: Vec::new(),
            observation_failures: Vec::new(),
            database_name: "database-secret".into(),
            result: Err(FixtureError::ObservationTimeout),
        }],
        drain: Err(FixtureError::ObservationTimeout),
    };
    let result = batter_test_support::finish(
        report.into_result().map_err(ProbeError::report),
        Err(ProbeError::new("observer-secret".into())),
    )
    .map_err(|error| Box::new(error) as _);
    let message = panic_message(result);
    for expected in [
        "Both",
        "body_failed=true",
        "acquisition_failures=1",
        "database_failures=1",
        "pool_failures=0",
        "observation_failures=0",
        "drain_failed=true",
        "cleanup:",
    ] {
        assert!(message.contains(expected), "missing {expected}");
    }
    assert!(!message.contains("secret"));
}

#[test]
fn unclassified_probe_error_contents_remain_redacted() {
    let message = panic_message(Err("native-secret".into()));
    assert!(message.contains("unclassified"));
    assert!(!message.contains("native-secret"));
}

#[test]
fn pending_probe_preserves_cause_and_prints_only_supplied_redacted_summary() {
    let error = ProbeError::pending(
        "native-secret".into(),
        "fixture completion pending; AwaitingRetry=1".into(),
    );
    assert!(error.0.to_string().contains("native-secret"));
    let message = panic_message(Err(Box::new(error)));
    assert!(message.contains("AwaitingRetry=1"));
    assert!(!message.contains("native-secret"));
}
