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
            pool_failures: vec![std::sync::Arc::new(sqlx::Error::Protocol(
                "pool-secret".into(),
            ))],
            database_name: "probe".into(),
            observation_failures: vec![std::sync::Arc::new(FixtureError::Observe(
                sqlx::Error::Protocol("observer-secret".into()),
            ))],
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

#[tokio::test]
async fn session_observation_rejects_unusable_budgets_before_acquisition() {
    use batter_sqlx::test_support::{FixtureError, SessionObserver};
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect_lazy("postgres://probe@localhost/unused")
        .unwrap();
    for bound in [std::time::Duration::ZERO, std::time::Duration::MAX] {
        assert!(matches!(
            SessionObserver::new(pool.clone(), bound),
            Err(FixtureError::InvalidBudget)
        ));
    }
    assert_eq!(pool.size(), 0);
    pool.close().await;
}

#[test]
fn progress_debug_exposes_phase_and_count_without_names_or_native_contents() {
    use batter_sqlx::test_support::{CleanupPhase, DatabaseProgress, FixtureError};
    let progress = DatabaseProgress {
        database_name: "database-secret".into(),
        phase: CleanupPhase::AwaitingRetry,
        observation_failures: vec![std::sync::Arc::new(FixtureError::Observe(
            sqlx::Error::Protocol("observer-secret".into()),
        ))],
    };
    let debug = format!("{progress:?}");
    assert!(debug.contains("AwaitingRetry"));
    assert!(debug.contains("observation_failures: 1"));
    assert!(!debug.contains("secret"));
}

fn successful_database_report() -> batter_sqlx::test_support::FixtureReport<(), std::io::Error> {
    use batter_sqlx::test_support::{DatabaseCleanup, FixtureReport};
    FixtureReport {
        body: Ok(()),
        acquisitions: vec![Ok(())],
        databases: vec![DatabaseCleanup {
            database_name: "probe".into(),
            pool_failures: vec![],
            observation_failures: vec![],
            result: Ok(()),
        }],
        drain: Ok(()),
    }
}

fn assert_database_failure(
    report: &batter_sqlx::test_support::FixtureReport<(), std::io::Error>,
    expected: &str,
) {
    assert!(!report.is_ok());
    let summary = report.to_string();
    assert!(summary.contains("body_failed=false"));
    assert!(summary.contains(expected), "{summary}");
    assert!(!summary.contains("secret"));
}

#[test]
fn handled_pool_error_alone_keeps_report_unsuccessful_and_native_source_identity() {
    use std::{error::Error, sync::Arc};
    let cause = Arc::new(sqlx::Error::Protocol("pool-secret".into()));
    let mut report = successful_database_report();
    report.databases[0].pool_failures.push(cause.clone());
    assert_database_failure(
        &report,
        "database_failures=0, pool_failures=1, observation_failures=0",
    );
    assert!(std::ptr::eq(
        report
            .source()
            .unwrap()
            .downcast_ref::<sqlx::Error>()
            .unwrap(),
        cause.as_ref(),
    ));
    assert!(report.into_result().is_err());
}

#[test]
fn recovered_observation_alone_keeps_report_unsuccessful_and_source_identity() {
    use batter_sqlx::test_support::FixtureError;
    use std::{error::Error, sync::Arc};
    let cause = Arc::new(FixtureError::Observe(sqlx::Error::Protocol(
        "observer-secret".into(),
    )));
    let mut report = successful_database_report();
    report.databases[0].observation_failures.push(cause.clone());
    assert_database_failure(
        &report,
        "database_failures=0, pool_failures=0, observation_failures=1",
    );
    assert!(std::ptr::eq(
        report
            .source()
            .unwrap()
            .downcast_ref::<FixtureError>()
            .unwrap(),
        cause.as_ref(),
    ));
    assert!(report.into_result().is_err());
}

#[test]
fn database_source_prioritizes_acquisition_then_observation_then_cleanup() {
    use batter_sqlx::test_support::FixtureError;
    use std::{error::Error, sync::Arc};
    let pool = Arc::new(sqlx::Error::PoolClosed);
    let observation = Arc::new(FixtureError::ObservationTimeout);
    let mut report = successful_database_report();
    report.databases[0].pool_failures.push(pool.clone());
    report.databases[0]
        .observation_failures
        .push(observation.clone());
    report.databases[0].result = Err(FixtureError::ObservationTarget);
    assert_database_failure(
        &report,
        "database_failures=1, pool_failures=1, observation_failures=1",
    );
    assert!(std::ptr::eq(
        report
            .source()
            .unwrap()
            .downcast_ref::<sqlx::Error>()
            .unwrap(),
        pool.as_ref(),
    ));
    report.databases[0].pool_failures.clear();
    assert!(std::ptr::eq(
        report
            .source()
            .unwrap()
            .downcast_ref::<FixtureError>()
            .unwrap(),
        observation.as_ref(),
    ));
    report.databases[0].observation_failures.clear();
    assert_database_failure(
        &report,
        "database_failures=1, pool_failures=0, observation_failures=0",
    );
    assert!(matches!(
        report.source().unwrap().downcast_ref::<FixtureError>(),
        Some(FixtureError::ObservationTarget)
    ));
}
