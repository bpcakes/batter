use batter_sqlx::test_support::{ConnectionPlan, MigrationBundle, MigrationInput, template_spec};
use sqlx::postgres::PgPoolOptions;

#[test]
fn budgets_include_standalone_capacity_and_reject_overflow() {
    let plan = ConnectionPlan::new(
        vec![
            PgPoolOptions::new().max_connections(2),
            PgPoolOptions::new().max_connections(3),
        ],
        2,
    )
    .unwrap();
    assert!(plan.validate(7).is_ok());
    assert!(plan.validate(6).is_err());
    assert!(ConnectionPlan::new(vec![PgPoolOptions::new().max_connections(1)], u32::MAX).is_err());
    assert!(ConnectionPlan::new(vec![], 1).is_err());
    assert!(ConnectionPlan::new(vec![PgPoolOptions::new().max_connections(0)], 0).is_err());
}

#[test]
fn template_identity_tracks_exact_ordered_inputs() {
    let first = [MigrationInput {
        identity: "1",
        kind: "up",
        sql: b"SELECT 1",
    }];
    let changed = [MigrationInput {
        identity: "1",
        kind: "up",
        sql: b"SELECT 2",
    }];
    let second = [MigrationInput {
        identity: "2",
        kind: "up",
        sql: b"SELECT 3",
    }];
    let spec = |a, b, revision| {
        template_spec(
            &[
                MigrationBundle {
                    identity: "a",
                    migrations: a,
                },
                MigrationBundle {
                    identity: "b",
                    migrations: b,
                },
            ],
            revision,
        )
    };
    let original = spec(&first, &second, "1");
    assert_eq!(original, spec(&first, &second, "1"));
    assert_ne!(original, spec(&changed, &second, "1"));
    assert_ne!(original, spec(&first, &second, "2"));
    let renamed = [MigrationInput {
        identity: "renamed",
        ..first[0]
    }];
    let retyped = [MigrationInput {
        kind: "down",
        ..first[0]
    }];
    assert_ne!(original, spec(&renamed, &second, "1"));
    assert_ne!(original, spec(&retyped, &second, "1"));
    assert_ne!(
        original,
        template_spec(
            &[
                MigrationBundle {
                    identity: "b",
                    migrations: &second
                },
                MigrationBundle {
                    identity: "a",
                    migrations: &first
                }
            ],
            "1"
        )
    );
}

#[test]
fn fixture_errors_redact_formatting_but_retain_native_causes() {
    use batter_sqlx::test_support::FixtureError;
    use std::error::Error;
    let error = FixtureError::Connect {
        source: sqlx::Error::Protocol("synthetic-secret".into()),
        cleanup: None,
    };
    assert!(!format!("{error:?}: {error}").contains("synthetic-secret"));
    assert!(
        matches!(error.source().and_then(|cause| cause.downcast_ref::<sqlx::Error>()), Some(sqlx::Error::Protocol(message)) if message == "synthetic-secret")
    );
}

#[test]
fn synthetic_report_preserves_every_failure_branch_and_redacts_contents() {
    use batter_sqlx::test_support::{
        AcquisitionFailure, BodyFailure, DatabaseCleanup, FixtureError, FixtureReport,
    };
    let producer = std::sync::Arc::new(postgres_test_harness::Error::InvalidConfiguration {
        reason: "producer-secret",
    });
    let report = FixtureReport::<(), _> {
        body: Err(BodyFailure::Returned(sqlx::Error::Protocol(
            "body-secret".into(),
        ))),
        acquisitions: vec![Err(AcquisitionFailure::Native(producer.clone()))],
        databases: vec![DatabaseCleanup {
            database_name: "probe".into(),
            result: Err(FixtureError::Observe(sqlx::Error::Protocol(
                "cleanup-secret".into(),
            ))),
        }],
        drain: Err(FixtureError::ObservationTimeout),
    };
    // Synthetic boundary errors verify aggregation only; live tests supply the
    // actual independent body/lease-cleanup failures.
    let formatted = format!("{report}: {report:?}");
    assert!(!formatted.contains("secret"));
    let report = report
        .into_result()
        .expect_err("every failure must survive");
    assert!(
        matches!(report.body, Err(BodyFailure::Returned(sqlx::Error::Protocol(ref message))) if message == "body-secret")
    );
    assert!(
        matches!(&report.databases[0].result, Err(FixtureError::Observe(sqlx::Error::Protocol(message))) if message == "cleanup-secret")
    );
    assert!(matches!(
        report.drain,
        Err(FixtureError::ObservationTimeout)
    ));
    assert!(
        matches!(&report.acquisitions[0], Err(AcquisitionFailure::Native(error)) if std::sync::Arc::ptr_eq(error, &producer))
    );
}
