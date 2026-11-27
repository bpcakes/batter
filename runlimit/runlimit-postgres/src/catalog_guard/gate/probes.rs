//! Each sqlx entry point `clippy.toml` disallows, used once under an
//! expectation that Clippy rejects it, plus the call shapes the gate must
//! resolve: a renamed import, a trait call through UFCS and on a generic
//! executor, a function pointer and a local macro. rustc ignores Clippy
//! expectations, and a Clippy run with `-D warnings` fails on an unfulfilled
//! one, so an entry that stops resolving or a shape the gate misses fails
//! verification. Nothing here runs. sqlx's query macros need a database to
//! expand, so `clippy.toml` lists them without a probe.

#![allow(
    dead_code,
    clippy::let_underscore_future,
    clippy::unused_async,
    reason = "probes are linted, never run"
)]

use std::path::Path;

use sqlx::{
    Connection as _, Executor as _, PgConnection, PgPool, Postgres, SqlSafeStr as _,
    migrate::{Migrate as _, MigrateDatabase as _, MigrationSource as _, ResolveConfig},
    postgres::{PgArguments, PgPoolCopyExt as _},
};

const SQL: &str = "SELECT 1";
const COPY: &str = "COPY t FROM STDIN";

#[expect(clippy::disallowed_methods, reason = "probe")]
fn query() {
    let _ = sqlx::query::<Postgres>(SQL);
}

#[expect(clippy::disallowed_methods, reason = "probe")]
fn query_with() {
    let _ = sqlx::query_with::<Postgres, _>(SQL, PgArguments::default());
}

#[expect(clippy::disallowed_methods, reason = "probe")]
fn query_with_result() {
    let _ = sqlx::__query_with_result::<Postgres, _>(SQL, Ok(PgArguments::default()));
}

#[expect(clippy::disallowed_methods, reason = "probe")]
fn query_as() {
    let _ = sqlx::query_as::<Postgres, (i64,)>(SQL);
}

#[expect(clippy::disallowed_methods, reason = "probe")]
fn query_as_with() {
    let _ = sqlx::query_as_with::<Postgres, (i64,), _>(SQL, PgArguments::default());
}

#[expect(clippy::disallowed_methods, reason = "probe")]
fn query_scalar() {
    let _ = sqlx::query_scalar::<Postgres, i64>(SQL);
}

#[expect(clippy::disallowed_methods, reason = "probe")]
fn query_scalar_with() {
    let _ = sqlx::query_scalar_with::<Postgres, i64, _>(SQL, PgArguments::default());
}

#[expect(clippy::disallowed_methods, reason = "probe")]
fn query_scalar_with_result() {
    let _ = sqlx::__query_scalar_with_result::<Postgres, i64, _>(SQL, Ok(PgArguments::default()));
}

#[expect(clippy::disallowed_methods, reason = "probe")]
fn raw_sql() {
    let _ = sqlx::raw_sql(SQL);
}

#[expect(clippy::disallowed_methods, reason = "probe")]
fn execute(connection: &mut PgConnection) {
    let _ = connection.execute(SQL);
}

#[expect(clippy::disallowed_methods, reason = "probe")]
fn execute_many(connection: &mut PgConnection) {
    let _ = connection.execute_many(SQL);
}

#[expect(clippy::disallowed_methods, reason = "probe")]
fn fetch(connection: &mut PgConnection) {
    let _ = connection.fetch(SQL);
}

#[expect(clippy::disallowed_methods, reason = "probe")]
fn fetch_many(connection: &mut PgConnection) {
    let _ = connection.fetch_many(SQL);
}

#[expect(clippy::disallowed_methods, reason = "probe")]
fn fetch_all(connection: &mut PgConnection) {
    let _ = connection.fetch_all(SQL);
}

#[expect(clippy::disallowed_methods, reason = "probe")]
fn fetch_one(connection: &mut PgConnection) {
    let _ = connection.fetch_one(SQL);
}

#[expect(clippy::disallowed_methods, reason = "probe")]
fn fetch_optional(connection: &mut PgConnection) {
    let _ = connection.fetch_optional(SQL);
}

// The text parameter is a `SqlStr`, which the type expectation covers.
#[expect(clippy::disallowed_methods, reason = "probe")]
#[expect(clippy::disallowed_types, reason = "probe")]
fn prepare(connection: &mut PgConnection, sql: sqlx::SqlStr) {
    let _ = connection.prepare(sql);
}

#[expect(clippy::disallowed_methods, reason = "probe")]
#[expect(clippy::disallowed_types, reason = "probe")]
fn prepare_with(connection: &mut PgConnection, sql: sqlx::SqlStr) {
    let _ = connection.prepare_with(sql, &[]);
}

#[expect(clippy::disallowed_methods, reason = "probe")]
#[expect(clippy::disallowed_types, reason = "probe")]
fn describe(connection: &mut PgConnection, sql: sqlx::SqlStr) {
    let _ = connection.describe(sql);
}

#[expect(clippy::disallowed_methods, reason = "probe")]
fn connection_begin_with(connection: &mut PgConnection) {
    let _ = connection.begin_with("BEGIN");
}

#[expect(clippy::disallowed_methods, reason = "probe")]
fn pool_begin_with(pool: &PgPool) {
    let _ = pool.begin_with("BEGIN");
}

#[expect(clippy::disallowed_methods, reason = "probe")]
fn pool_try_begin_with(pool: &PgPool) {
    let _ = pool.try_begin_with("BEGIN");
}

#[expect(clippy::disallowed_methods, reason = "probe")]
fn connection_copy_in_raw(connection: &mut PgConnection) {
    let _ = connection.copy_in_raw(COPY);
}

#[expect(clippy::disallowed_methods, reason = "probe")]
fn connection_copy_out_raw(connection: &mut PgConnection) {
    let _ = connection.copy_out_raw(COPY);
}

#[expect(clippy::disallowed_methods, reason = "probe")]
fn pool_copy_in_raw(pool: &PgPool) {
    let _ = pool.copy_in_raw(COPY);
}

#[expect(clippy::disallowed_methods, reason = "probe")]
fn pool_copy_out_raw(pool: &PgPool) {
    let _ = pool.copy_out_raw(COPY);
}

#[expect(clippy::disallowed_methods, reason = "probe")]
fn into_sql_str() {
    let _ = SQL.into_sql_str();
}

#[expect(clippy::disallowed_methods, reason = "probe")]
fn execute_sql() {
    let _ = <&str as sqlx::Execute<'_, Postgres>>::sql(SQL);
}

// `None` needs no `SqlStr`, so only the method can fulfil the expectation.
#[expect(clippy::disallowed_methods, reason = "probe")]
fn transaction_begin(connection: &mut PgConnection) {
    let _ = sqlx::Transaction::<Postgres>::begin(connection, None);
}

#[expect(clippy::disallowed_methods, reason = "probe")]
fn migrator_new() {
    let _ = sqlx::migrate::Migrator::new(Path::new("migrations"));
}

#[expect(clippy::disallowed_methods, reason = "probe")]
fn migration_source_resolve() {
    let _ = Path::new("migrations").resolve();
}

#[expect(clippy::disallowed_methods, reason = "probe")]
fn resolve_blocking() {
    let _ = sqlx::migrate::resolve_blocking(Path::new("migrations"));
}

#[expect(clippy::disallowed_methods, reason = "probe")]
fn resolve_blocking_with_config() {
    let _ =
        sqlx::migrate::resolve_blocking_with_config(Path::new("migrations"), &ResolveConfig::new());
}

#[expect(clippy::disallowed_methods, reason = "probe")]
fn dangerous_set_table_name() {
    let mut migrator = sqlx::migrate::Migrator::with_migrations(Vec::new());
    let _ = migrator.dangerous_set_table_name("migrations");
}

#[expect(clippy::disallowed_methods, reason = "probe")]
fn create_schema() {
    let mut migrator = sqlx::migrate::Migrator::with_migrations(Vec::new());
    let _ = migrator.create_schema("schema");
}

#[expect(clippy::disallowed_methods, reason = "probe")]
fn create_schema_if_not_exists(connection: &mut PgConnection) {
    let _ = connection.create_schema_if_not_exists("schema");
}

#[expect(clippy::disallowed_methods, reason = "probe")]
fn ensure_migrations_table(connection: &mut PgConnection) {
    let _ = connection.ensure_migrations_table("migrations");
}

#[expect(clippy::disallowed_methods, reason = "probe")]
fn dirty_version(connection: &mut PgConnection) {
    let _ = connection.dirty_version("migrations");
}

#[expect(clippy::disallowed_methods, reason = "probe")]
fn list_applied_migrations(connection: &mut PgConnection) {
    let _ = connection.list_applied_migrations("migrations");
}

#[expect(clippy::disallowed_methods, reason = "probe")]
fn migrate_lock(connection: &mut PgConnection) {
    let _ = connection.lock();
}

#[expect(clippy::disallowed_methods, reason = "probe")]
fn migrate_unlock(connection: &mut PgConnection) {
    let _ = connection.unlock();
}

// The migration parameter names `Migration`, which the type expectation covers.
#[expect(clippy::disallowed_methods, reason = "probe")]
#[expect(clippy::disallowed_types, reason = "probe")]
fn migrate_apply(connection: &mut PgConnection, migration: &sqlx::migrate::Migration) {
    let _ = connection.apply("migrations", migration);
}

#[expect(clippy::disallowed_methods, reason = "probe")]
#[expect(clippy::disallowed_types, reason = "probe")]
fn migrate_revert(connection: &mut PgConnection, migration: &sqlx::migrate::Migration) {
    let _ = connection.revert("migrations", migration);
}

#[expect(clippy::disallowed_methods, reason = "probe")]
#[expect(clippy::disallowed_types, reason = "probe")]
fn migrate_skip(connection: &mut PgConnection, migration: &sqlx::migrate::Migration) {
    let _ = connection.skip("migrations", migration);
}

#[expect(clippy::disallowed_methods, reason = "probe")]
fn create_database() {
    let _ = Postgres::create_database("postgresql://localhost/database");
}

#[expect(clippy::disallowed_methods, reason = "probe")]
fn database_exists() {
    let _ = Postgres::database_exists("postgresql://localhost/database");
}

#[expect(clippy::disallowed_methods, reason = "probe")]
fn drop_database() {
    let _ = Postgres::drop_database("postgresql://localhost/database");
}

#[expect(clippy::disallowed_methods, reason = "probe")]
fn force_drop_database() {
    let _ = Postgres::force_drop_database("postgresql://localhost/database");
}

#[expect(clippy::disallowed_types, reason = "probe")]
fn listener() {
    let _: Option<sqlx::postgres::PgListener> = None;
}

#[expect(clippy::disallowed_types, reason = "probe")]
fn advisory_lock() {
    let _ = sqlx::postgres::PgAdvisoryLock::new("key");
}

#[expect(clippy::disallowed_types, reason = "probe")]
fn assert_sql_safe() {
    let _: Option<sqlx::AssertSqlSafe<String>> = None;
}

#[expect(clippy::disallowed_types, reason = "probe")]
fn sql_str() {
    let _ = sqlx::SqlStr::from_static(SQL);
}

#[expect(clippy::disallowed_types, reason = "probe")]
fn query_builder() {
    let _ = sqlx::QueryBuilder::<Postgres>::new(SQL);
}

#[expect(clippy::disallowed_types, reason = "probe")]
fn raw_sql_type() {
    let _: Option<sqlx::RawSql> = None;
}

#[expect(clippy::disallowed_types, reason = "probe")]
fn migration() {
    let _: Option<sqlx::migrate::Migration> = None;
}

#[expect(clippy::disallowed_methods, reason = "probe")]
fn renamed_import() {
    use sqlx::query as run;
    let _ = run::<Postgres>(SQL);
}

#[expect(clippy::disallowed_methods, reason = "probe")]
fn fully_qualified(pool: &PgPool) {
    let _ = <&PgPool as sqlx::Executor<'_>>::execute(pool, SQL);
}

#[expect(clippy::disallowed_methods, reason = "probe")]
fn generic_executor<'e, E: sqlx::Executor<'e, Database = Postgres>>(executor: E) {
    let _ = executor.fetch_all(SQL);
}

#[expect(clippy::disallowed_methods, reason = "probe")]
fn function_pointer() {
    let _: fn(&'static str) -> sqlx::query::Query<'static, Postgres, PgArguments> = sqlx::query;
}

#[expect(clippy::disallowed_methods, reason = "probe")]
fn local_macro(connection: &mut PgConnection) {
    macro_rules! hidden {
        ($connection:expr) => {
            $connection.execute(SQL)
        };
    }
    let _ = hidden!(connection);
}
