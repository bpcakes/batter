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
    let mut findings = Findings::default();
    for history in History::ALL {
        let schema = format!(
            "runlimit_guard_{history:?}_{}_{started}",
            std::process::id()
        )
        .to_ascii_lowercase();
        administer(&admin, format!("CREATE SCHEMA {schema}")).await;
        // A setup failure is kept like a preparation failure, so the schema
        // is dropped before anything is reported.
        if let Err(failure) = check_history(&url, &schema, history, &inventory, &mut findings).await
        {
            findings
                .failures
                .push(format!("{history:?} setup: {failure}"));
        }
        administer(&admin, format!("DROP SCHEMA {schema} CASCADE")).await;
    }
    admin.close().await;
    assert!(
        findings.failures.is_empty(),
        "statements PostgreSQL cannot resolve:\n{:#?}",
        findings.failures
    );
    let all: BTreeSet<String> = inventory.into_iter().map(|(name, _)| name).collect();
    assert_eq!(
        findings.prepared, all,
        "every statement is prepared in some schema"
    );
}

/// What the check found: each statement prepared, and each failure.
#[derive(Default)]
struct Findings {
    prepared: BTreeSet<String>,
    failures: Vec<String>,
}

/// Connects inside the empty `schema`, checks `history` there, and closes
/// the connection whatever the check found.
async fn check_history(
    url: &str,
    schema: &str,
    history: History,
    inventory: &[(String, Statement)],
    findings: &mut Findings,
) -> Result<(), String> {
    let search_path = format!("{schema}, pg_catalog");
    let options = PgConnectOptions::from_str(url)
        .map_err(|error| format!("parse the test database URL: {error}"))?
        .options([("search_path", search_path.as_str())]);
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .map_err(|error| format!("connect inside the schema: {error}"))?;
    let checked = prepare_history(&pool, schema, history, inventory, findings).await;
    pool.close().await;
    checked
}

/// Verifies the search path, migrates `history` and prepares each statement
/// that runs against it.
async fn prepare_history(
    pool: &PgPool,
    schema: &str,
    history: History,
    inventory: &[(String, Statement)],
    findings: &mut Findings,
) -> Result<(), String> {
    let schemas = current_schemas(pool)
        .await
        .map_err(|error| format!("read the search path: {error}"))?;
    if schemas != [schema, "pg_catalog"] {
        return Err(format!("the connection searches {schemas:?}"));
    }
    history
        .migrate(pool)
        .await
        .map_err(|error| format!("install the schema: {error}"))?;
    for (name, statement) in inventory {
        if History::of(*statement).contains(&history) {
            if let Err(error) = prepare(pool, *statement).await {
                findings
                    .failures
                    .push(format!("{history:?} {name}: {error}"));
            }
            findings.prepared.insert(name.clone());
        }
    }
    Ok(())
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
async fn current_schemas(pool: &PgPool) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar("SELECT pg_catalog.current_schemas(false)")
        .fetch_one(pool)
        .await
}
