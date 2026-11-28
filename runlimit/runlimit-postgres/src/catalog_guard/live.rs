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
    collections::BTreeSet,
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

/// A migration history, which installs alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum History {
    FixedWindow,
    Gcra,
    Attempts,
}

impl History {
    const ALL: [Self; 3] = [Self::FixedWindow, Self::Gcra, Self::Attempts];

    /// The histories whose schema `statement` runs against: its family's,
    /// or every one for the session statements all transactions share. The
    /// match is exhaustive, so a new family names its history here.
    const fn of(statement: Statement) -> &'static [Self] {
        match statement {
            Statement::FixedWindow(_) => &[Self::FixedWindow],
            Statement::Gcra(_) => &[Self::Gcra],
            Statement::Attempts(_) => &[Self::Attempts],
            Statement::Session(_) => &Self::ALL,
        }
    }

    async fn migrate(self, pool: &PgPool) -> Result<(), MigrateError> {
        match self {
            Self::FixedWindow => PostgresLimiter::new(pool.clone()).migrate().await,
            Self::Gcra => PostgresGcraLimiter::new(pool.clone()).migrate().await,
            Self::Attempts => PostgresAttemptLimiter::new(pool.clone()).migrate().await,
        }
    }
}

// The live suites' `tests/support` fixture is an integration-test module,
// out of a unit test's reach, so this check provisions its schema the same
// way: an empty schema, searched before `pg_catalog`, verified on connect.
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
    let inventory = statements::all();
    let mut prepared = BTreeSet::new();
    let mut unresolved = Vec::new();
    for history in History::ALL {
        let schema = format!(
            "runlimit_guard_{history:?}_{}_{started}",
            std::process::id()
        )
        .to_ascii_lowercase();
        administer(&admin, format!("CREATE SCHEMA {schema}")).await;
        let search_path = format!("{schema}, pg_catalog");
        let options = PgConnectOptions::from_str(&url)
            .expect("the test database URL parses")
            .options([("search_path", search_path.as_str())]);
        let pool = PgPoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .expect("connect inside the empty schema");
        assert_eq!(
            current_schemas(&pool).await,
            [schema.as_str(), "pg_catalog"]
        );
        history
            .migrate(&pool)
            .await
            .unwrap_or_else(|error| panic!("{history:?} installs its schema: {error}"));
        for (name, statement) in &inventory {
            if History::of(*statement).contains(&history) {
                if let Err(error) = prepare(&pool, *statement).await {
                    unresolved.push(format!("{history:?} {name}: {error}"));
                }
                prepared.insert(name.clone());
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
    let all: BTreeSet<String> = inventory.into_iter().map(|(name, _)| name).collect();
    assert_eq!(prepared, all, "every statement is prepared in some schema");
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

/// Creates or drops the schema one history migrates into.
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

/// The schemas `pool`'s connections search, as the shared fixture checks.
#[expect(
    clippy::disallowed_methods,
    reason = "the live check verifies its search path"
)]
async fn current_schemas(pool: &PgPool) -> Vec<String> {
    sqlx::query_scalar("SELECT pg_catalog.current_schemas(false)")
        .fetch_one(pool)
        .await
        .expect("read the check's search path")
}
