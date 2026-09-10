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
