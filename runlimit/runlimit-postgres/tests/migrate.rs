//! Live coverage of the migration path every `PostgreSQL` limiter shares.

#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "tests provision and inspect database state with their own SQL"
)]
mod support;

use std::{
    collections::{BTreeMap, BTreeSet},
    time::Duration,
};

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

impl Storage {
    /// The statement family whose tables this history installs, as
    /// `src/catalog_guard/migrated_columns.txt` names it.
    const fn family(self) -> &'static str {
        match self {
            Self::FixedWindow => "fixed_window",
            Self::Gcra => "gcra",
            Self::Attempts => "attempts",
        }
    }
}

/// The column inventory the statement guard checks field references against.
const MIGRATED_COLUMNS: &str = include_str!("../src/catalog_guard/migrated_columns.txt");

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

/// The tables `storage`'s history leaves in `fixture`'s schema, one line
/// each, as `src/catalog_guard/migrated_columns.txt` lists them.
async fn migrated_tables(
    storage: Storage,
    fixture: &IsolatedSchema,
) -> Result<Vec<String>, String> {
    storage
        .migrate(&fixture.pool)
        .await
        .map_err(|error| format!("installs its schema: {error}"))?;
    let columns: Vec<(String, String)> = sqlx::query_as(
        "SELECT c.relname::text, a.attname::text FROM pg_catalog.pg_class AS c \
         JOIN pg_catalog.pg_namespace AS n ON n.oid = c.relnamespace \
         JOIN pg_catalog.pg_attribute AS a ON a.attrelid = c.oid \
         WHERE n.nspname = $1 AND c.relkind IN ('r', 'p') \
         AND c.relname <> '_sqlx_migrations' AND a.attnum > 0 AND NOT a.attisdropped",
    )
    .bind(&fixture.schema)
    .fetch_all(&fixture.admin_pool)
    .await
    .map_err(|error| format!("read the migrated tables' columns: {error}"))?;
    let mut tables = BTreeMap::<String, BTreeSet<String>>::new();
    for (table, column) in columns {
        tables.entry(table).or_default().insert(column);
    }
    Ok(tables
        .into_iter()
        .map(|(table, columns)| {
            let columns: Vec<String> = columns.into_iter().collect();
            format!("{} {table} {}", storage.family(), columns.join(" "))
        })
        .collect())
}

// The statement guard cannot read what a migration's functions and
// triggers leave behind, so PostgreSQL reports each history's tables.
#[tokio::test]
#[ignore = "requires RUNLIMIT_POSTGRES_TEST_DATABASE_URL"]
async fn the_statement_guard_lists_the_columns_each_history_leaves() {
    let mut migrated = Vec::new();
    for storage in [Storage::Attempts, Storage::FixedWindow, Storage::Gcra] {
        // Each history installs alone, into its own empty schema, which is
        // dropped before any failure is reported.
        let fixture = IsolatedSchema::create("inventory", 1).await;
        let tables = migrated_tables(storage, &fixture).await;
        fixture.teardown().await;
        migrated.extend(tables.unwrap_or_else(|failure| panic!("{storage:?}: {failure}")));
    }
    let listed: Vec<&str> = MIGRATED_COLUMNS
        .lines()
        .filter(|line| !line.starts_with('#') && !line.trim().is_empty())
        .collect();
    assert_eq!(
        listed,
        migrated,
        "src/catalog_guard/migrated_columns.txt must list each history's tables:\n{}",
        migrated.join("\n")
    );
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
