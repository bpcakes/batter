#![allow(clippy::unwrap_used, reason = "test assertions")]
use super::*;
use std::{error::Error, time::Duration};
pub(crate) struct Pause {
    pub entered: tokio::sync::Notify,
    pub release: tokio::sync::Notify,
}
#[test]
fn construction_and_server_refusals() {
    for text in ["", "a\n", "\u{7f}"] {
        assert!(Actor::new(text).is_err());
        assert!(ChangeReason::new(text).is_err());
    }
    assert!(Actor::new(&"a".repeat(129)).is_err());
    assert!(ChangeReason::new(&"a".repeat(1025)).is_err());
    assert!(
        StoreTiming::new(
            Duration::ZERO,
            Duration::from_secs(1),
            Duration::from_secs(1),
            Duration::from_secs(1)
        )
        .is_err()
    );
    assert_eq!(
        errors::check_server(179999).unwrap_err().class(),
        ErrorClass::UnsupportedServer
    );
    assert!(errors::check_server(180000).is_ok());
    assert_eq!(
        errors::check_primary(true).unwrap_err().class(),
        ErrorClass::Standby
    );
}
#[test]
fn diagnostics_do_not_print_causes() {
    let marker = "secret-marker";
    let error: StoreError = sqlx::Error::Protocol(marker.into()).into();
    for text in [
        format!("{error}"),
        format!("{error:?}"),
        format!("{error:#?}"),
    ] {
        assert!(!text.contains(marker));
    }
    assert!(
        error
            .source()
            .unwrap()
            .source()
            .unwrap()
            .to_string()
            .contains(marker)
    );
    let actor = Actor::new(marker).unwrap();
    assert!(!format!("{actor:?}").contains(marker));
}
#[path = "../tests/support/fixture.rs"]
mod fixture;

#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn stalled_counter_owner_expires_and_kill_switch_progresses() {
    use fixture::*;
    use std::sync::Arc;
    let f = Fixture::migrated().await;
    f.seed("halt", "switch", "switch", "live").await;
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(f.lease.database_url())
        .await
        .unwrap();
    let pause = Arc::new(Pause {
        entered: tokio::sync::Notify::new(),
        release: tokio::sync::Notify::new(),
    });
    let mut paused = PooledStore::new(pool.clone(), timing());
    paused.pause = Some(pause.clone());
    let before = tokio::time::Instant::now();
    let task = tokio::spawn(async move {
        paused
            .write(
                Write::explicit(
                    key("paused"),
                    ExplicitValue::ModeToggle(batter_flags_core::Toggle::On),
                    change(None),
                ),
                before + Duration::from_millis(250),
            )
            .await
    });
    tokio::time::timeout(Duration::from_secs(2), pause.entered.notified())
        .await
        .unwrap();
    let kill = f.store.write(
        Write::kill(
            key("halt"),
            batter_flags_core::SwitchState::Engaged,
            change(None),
        ),
        deadline(),
    );
    let (stalled, killed) = tokio::join!(task, kill);
    assert_eq!(stalled.unwrap().unwrap_err().class(), ErrorClass::Deadline);
    assert!(matches!(killed.unwrap(),WriteOutcome::Written { version, .. } if version.get()==1));
    assert!(before.elapsed() < Duration::from_secs(3));
    assert_eq!(pool.size(), 0);
    pool.close().await;
    f.close().await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn dropping_stalled_write_retires_and_does_not_commit() {
    use fixture::*;
    use std::sync::Arc;
    let f = Fixture::migrated().await;
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(f.lease.database_url())
        .await
        .unwrap();
    let pause = Arc::new(Pause {
        entered: tokio::sync::Notify::new(),
        release: tokio::sync::Notify::new(),
    });
    let mut paused = PooledStore::new(pool.clone(), timing());
    paused.pause = Some(pause.clone());
    let task = tokio::spawn(async move {
        paused
            .write(
                Write::explicit(
                    key("paused"),
                    ExplicitValue::ModeToggle(batter_flags_core::Toggle::On),
                    change(None),
                ),
                deadline(),
            )
            .await
    });
    tokio::time::timeout(Duration::from_secs(2), pause.entered.notified())
        .await
        .unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert_eq!(pool.size(), 0);
    assert!(
        matches!(f.store.write(Write::explicit(key("following"),ExplicitValue::ModeToggle(batter_flags_core::Toggle::On),change(None)),deadline()).await.unwrap(),WriteOutcome::WrittenUndeclared { version, .. } if version.get()==1)
    );
    pool.close().await;
    f.close().await;
}

#[test]
fn every_error_wrapper_redacts_both_formats_and_preserves_causes() {
    use batter_sqlx::{PgScopeFailure, PgSnapshotError, PgTransactionError};
    use errors::ErrorKind;
    let native = || sqlx::Error::Protocol("retained-private-marker".into());
    let errors = [
        StoreError(ErrorKind::Transaction(PgTransactionError::from(native()))),
        StoreError(ErrorKind::Scope(Box::new(PgScopeFailure::Recovery {
            application: native().into(),
            recovery: std::sync::Arc::new(native().into()),
        }))),
        StoreError(ErrorKind::Snapshot(Box::new(PgSnapshotError::Cleanup {
            inspection: native().into(),
            cleanup: native().into(),
        }))),
        StoreError(ErrorKind::Other(Box::new(std::io::Error::other(
            "retained-private-marker",
        )))),
    ];
    for error in errors {
        for text in [
            format!("{error}"),
            format!("{error:?}"),
            format!("{error:#?}"),
        ] {
            assert!(!text.contains("retained-private-marker"));
        }
        let mut current: &(dyn Error + 'static) = &error;
        let mut found = false;
        while let Some(source) = current.source() {
            found |= source.to_string().contains("retained-private-marker");
            current = source;
        }
        assert!(found);
    }
    for text in [
        format!("{ValidationError}"),
        format!("{ValidationError:?}"),
        format!("{ValidationError:#?}"),
        format!("{}", CommitUncertainty::Deadline),
        format!("{:?}", CommitUncertainty::Deadline),
        format!("{:#?}", CommitUncertainty::Deadline),
    ] {
        assert!(!text.contains("retained-private-marker"));
    }
}

#[test]
fn a_schema_observation_survives_a_failed_cleanup() {
    use crate::errors::ErrorKind;
    let behind = StoreError::schema_behind(0);
    assert_eq!(behind.installed_schema_revision(), Some(0));
    let wrapped = crate::RollbackFailure::deadline(StoreError::schema_behind(0));
    assert_eq!(wrapped.class(), ErrorClass::Database);
    assert_eq!(wrapped.installed_schema_revision(), Some(0));
    let snapshot = StoreError(ErrorKind::Snapshot(Box::new(
        batter_sqlx::PgSnapshotError::Inspection(StoreError::schema_behind(0)),
    )));
    assert_eq!(snapshot.installed_schema_revision(), Some(0));
    assert_eq!(
        StoreError::classified(ErrorClass::Database).installed_schema_revision(),
        None
    );
}

/// `load`, `preflight` and every operator read refuse a standby before they
/// read any schema or row: on a store that was never migrated, the standby
/// refusal wins over the schema refusal. The store's test hook makes the
/// recovery check read as a standby, because no suite runs a replica and a
/// read snapshot pins `search_path`, so no session can stand in for one; a
/// real replica's `pg_is_in_recovery()` stays unexercised.
#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn every_read_and_the_load_refuse_a_standby_before_reading_anything() {
    use fixture::*;
    fn class<T>(outcome: Result<T, StoreError>) -> Option<ErrorClass> {
        outcome.err().map(|error| error.class())
    }
    let f = Fixture::new().await;
    let halt = key("halt");
    let mut store = PooledStore::new(f.pool.clone(), timing());
    for (standby, expected) in [
        (true, ErrorClass::Standby),
        (false, ErrorClass::SchemaRevision),
    ] {
        store.standby = standby;
        let outcomes = [
            ("load", class(store.load(deadline()).await)),
            ("preflight", class(store.preflight(deadline()).await)),
            ("keys", class(store.keys(deadline()).await)),
            ("key", class(store.key(&halt, deadline()).await)),
            (
                "history",
                class(
                    store
                        .history(Some(&halt), batter_flags_core::Version::new(0), deadline())
                        .await,
                ),
            ),
            ("fleet", class(store.fleet(deadline()).await)),
            (
                "fleet_declaring",
                class(store.fleet_declaring(&halt, deadline()).await),
            ),
        ];
        for (operation, outcome) in outcomes {
            assert_eq!(outcome, Some(expected), "{operation} as standby={standby}");
        }
    }
    f.close().await;
}
