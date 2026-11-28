//! PostgreSQL's own check that every statement resolves where it runs.
//!
//! Each family's history migrates alone into an empty schema, and
//! PostgreSQL prepares the family's statements there: it parses each one
//! and resolves every name as it would at run time, without running it.
//!
//! The static guard proves that a qualified name it accepts is a column of
//! the one relation its receiver names, since each name is declared once
//! per statement. Whether PostgreSQL can resolve that receiver where it is
//! used (an alias's query block, `LATERAL` order, a join's operands, an
//! INSERT target outside its source query) is decided here instead.

use std::{
    str::FromStr as _,
    time::{SystemTime, UNIX_EPOCH},
};

use sqlx::{
    AssertSqlSafe, Executor as _, PgPool, SqlSafeStr,
    migrate::MigrateError,
    postgres::{PgConnectOptions, PgPoolOptions},
};

use crate::{
    PostgresGcraLimiter, PostgresLimiter,
    attempts::PostgresAttemptLimiter,
    statements::{self, Statement},
};

#[derive(Clone, Copy, Debug)]
enum Family {
    FixedWindow,
    Gcra,
    Attempts,
}

impl Family {
    /// Whether `statement` runs in this family's transactions: its own, and
    /// the session statements every family shares.
    const fn holds(self, statement: Statement) -> bool {
        matches!(
            (self, statement),
            (_, Statement::Session(_))
                | (Self::FixedWindow, Statement::FixedWindow(_))
                | (Self::Gcra, Statement::Gcra(_))
                | (Self::Attempts, Statement::Attempts(_))
        )
    }

    async fn migrate(self, pool: &PgPool) -> Result<(), MigrateError> {
        match self {
            Self::FixedWindow => PostgresLimiter::new(pool.clone()).migrate().await,
            Self::Gcra => PostgresGcraLimiter::new(pool.clone()).migrate().await,
            Self::Attempts => PostgresAttemptLimiter::new(pool.clone()).migrate().await,
        }
    }
}

#[tokio::test]
#[ignore = "requires RUNLIMIT_POSTGRES_TEST_DATABASE_URL"]
async fn postgres_resolves_every_statement_in_its_familys_schema() {
    let url = std::env::var("RUNLIMIT_POSTGRES_TEST_DATABASE_URL")
        .expect("RUNLIMIT_POSTGRES_TEST_DATABASE_URL is required for ignored tests");
    let admin = PgPool::connect(&url)
        .await
        .expect("connect to the test database");
    let started = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("the clock is after the epoch")
        .as_nanos();
    let mut unresolved = Vec::new();
    for family in [Family::FixedWindow, Family::Gcra, Family::Attempts] {
        let schema = format!("runlimit_guard_{family:?}_{}_{started}", std::process::id())
            .to_ascii_lowercase();
        administer(&admin, format!("CREATE SCHEMA {schema}")).await;
        let options = PgConnectOptions::from_str(&url)
            .expect("the test database URL parses")
            .options([("search_path", schema.as_str())]);
        let pool = PgPoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .expect("connect inside the empty schema");
        family
            .migrate(&pool)
            .await
            .unwrap_or_else(|error| panic!("{family:?} installs its schema: {error}"));
        for (name, statement) in statements::all() {
            if family.holds(statement)
                && let Err(error) = prepare(&pool, statement).await
            {
                unresolved.push(format!("{family:?} {name}: {error}"));
            }
        }
        pool.close().await;
        administer(&admin, format!("DROP SCHEMA {schema} CASCADE")).await;
    }
    admin.close().await;
    assert!(
        unresolved.is_empty(),
        "statements PostgreSQL cannot resolve:\n{unresolved:#?}"
    );
}

/// Asks PostgreSQL to prepare `statement`, resolving every name in it.
#[expect(
    clippy::disallowed_methods,
    reason = "PostgreSQL prepares each closed-set statement"
)]
async fn prepare(pool: &PgPool, statement: Statement) -> Result<(), sqlx::Error> {
    let mut connection = pool.acquire().await?;
    connection
        .prepare(SqlSafeStr::into_sql_str(statement.sql()))
        .await
        .map(drop)
}

/// Creates or drops the schema one family migrates into.
#[expect(
    clippy::disallowed_methods,
    reason = "the live check administers its own schema"
)]
async fn administer(admin: &PgPool, sql: String) {
    sqlx::query(AssertSqlSafe(sql))
        .execute(admin)
        .await
        .expect("administer the check's schema");
}
