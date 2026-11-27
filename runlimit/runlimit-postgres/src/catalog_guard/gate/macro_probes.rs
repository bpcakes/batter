//! Each sqlx query macro `clippy.toml` disallows, invoked once. Clippy
//! reports `disallowed_macros` outside any local lint scope, so an attribute
//! can neither expect nor suppress it. This module compiles only under
//! `--cfg runlimit_macro_probes`, and `scripts/check_runlimit_workspace.py`
//! runs Clippy with that cfg and requires each macro to be reported at its
//! probe. The macros expand offline from the package's committed `.sqlx`
//! metadata for `SELECT 1 AS one`, which `probe.sql` also holds; regenerate
//! it with `SQLX_OFFLINE=false SQLX_OFFLINE_DIR=<package>/.sqlx` against
//! PostgreSQL 18. Nothing here runs.

#![allow(dead_code, reason = "probes are linted, never run")]

/// The row the macro probes' metadata describes.
struct One {
    one: Option<i32>,
}

fn query_macro() {
    let _ = sqlx::query!("SELECT 1 AS one");
}

fn query_unchecked_macro() {
    let _ = sqlx::query_unchecked!("SELECT 1 AS one");
}

fn query_file_macro() {
    let _ = sqlx::query_file!("src/catalog_guard/gate/probe.sql");
}

fn query_file_unchecked_macro() {
    let _ = sqlx::query_file_unchecked!("src/catalog_guard/gate/probe.sql");
}

fn query_as_macro() {
    let _ = sqlx::query_as!(One, "SELECT 1 AS one");
}

fn query_as_unchecked_macro() {
    let _ = sqlx::query_as_unchecked!(One, "SELECT 1 AS one");
}

fn query_file_as_macro() {
    let _ = sqlx::query_file_as!(One, "src/catalog_guard/gate/probe.sql");
}

fn query_file_as_unchecked_macro() {
    let _ = sqlx::query_file_as_unchecked!(One, "src/catalog_guard/gate/probe.sql");
}

fn query_scalar_macro() {
    let _ = sqlx::query_scalar!("SELECT 1 AS one");
}

fn query_scalar_unchecked_macro() {
    let _ = sqlx::query_scalar_unchecked!("SELECT 1 AS one");
}

fn query_file_scalar_macro() {
    let _ = sqlx::query_file_scalar!("src/catalog_guard/gate/probe.sql");
}

fn query_file_scalar_unchecked_macro() {
    let _ = sqlx::query_file_scalar_unchecked!("src/catalog_guard/gate/probe.sql");
}

fn expand_query_macro() {
    let _ = sqlx::sqlx_macros::expand_query!(source = "SELECT 1 AS one");
}

#[sqlx::test]
async fn sqlx_test_attribute(_pool: sqlx::PgPool) {}
