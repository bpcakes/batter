//! Live coverage of the migration path every `PostgreSQL` limiter shares.

#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "tests provision and inspect database state with their own SQL"
)]
mod support;

use std::time::Duration;

use runlimit_postgres::{PostgresGcraLimiter, PostgresLimiter, attempts::PostgresAttemptLimiter};
use sqlx::{PgPool, migrate::MigrateError};
use support::IsolatedSchema;

#[derive(Clone, Copy, Debug)]
enum Storage {
    FixedWindow,
    Gcra,
    Attempts,
}

impl Storage {
    async fn migrate(self, pool: &PgPool) -> Result<(), MigrateError> {
        match self {
            Self::FixedWindow => PostgresLimiter::new(pool.clone()).migrate().await,
            Self::Gcra => PostgresGcraLimiter::new(pool.clone()).migrate().await,
            Self::Attempts => PostgresAttemptLimiter::new(pool.clone()).migrate().await,
        }
    }
}

async fn backend(pool: &PgPool) -> i32 {
    tokio::time::timeout(
        Duration::from_secs(2),
        sqlx::query_scalar("SELECT pg_backend_pid()").fetch_one(pool),
    )
    .await
    .expect("the only pool slot is available")
    .expect("the pooled connection is usable")
}

/// Counts the advisory locks `session` holds, read from another session.
async fn advisory_locks_held(admin_pool: &PgPool, session: i32) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM pg_catalog.pg_locks \
         WHERE pid = $1 AND locktype = 'advisory' AND granted",
    )
    .bind(session)
    .fetch_one(admin_pool)
    .await
    .expect("read the migration session's advisory locks")
}

#[tokio::test]
#[ignore = "requires RUNLIMIT_POSTGRES_TEST_DATABASE_URL"]
async fn every_migration_returns_its_session_only_after_success() {
    for storage in [Storage::FixedWindow, Storage::Gcra, Storage::Attempts] {
        // One connection: a retired session is replaced, a leaked one blocks.
        let fixture = IsolatedSchema::create("migrate", 1).await;
        let session = backend(&fixture.pool).await;

        storage
            .migrate(&fixture.pool)
            .await
            .unwrap_or_else(|error| panic!("{storage:?} installs its schema: {error}"));
        assert_eq!(
            backend(&fixture.pool).await,
            session,
            "{storage:?} returns its session after a successful migration"
        );
        // Advisory locks are reentrant within a session, so a repeated
        // migration on this session would succeed even with a leaked lock.
        assert_eq!(
            advisory_locks_held(&fixture.admin_pool, session).await,
            0,
            "{storage:?} must unlock the session it returns to the pool"
        );
        storage
            .migrate(&fixture.pool)
            .await
            .unwrap_or_else(|error| panic!("{storage:?} migration is repeatable: {error}"));

        // A checksum error occurs after SQLx takes its session advisory lock.
        sqlx::query("UPDATE _sqlx_migrations SET checksum = $1")
            .bind(vec![0_u8; 48])
            .execute(&fixture.pool)
            .await
            .expect("corrupt the applied checksums for the failure path");
        let error = storage
            .migrate(&fixture.pool)
            .await
            .expect_err("a modified migration checksum must fail");
        assert!(
            matches!(error, MigrateError::VersionMismatch(_)),
            "{storage:?} reported {error}"
        );
        assert_ne!(
            backend(&fixture.pool).await,
            session,
            "{storage:?} must not return a possibly locked migration session to the pool"
        );

        fixture.teardown().await;
    }
}
