//! Application-owned database policy. Native endpoint validation happens first.
use runledger_postgres::{PgSessionProfile, RunledgerDatabase};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use std::time::Duration;

/// Caller-owned fixture construction; the fixture must await pool closure.
/// Production startup uses `configured`.
/// This reference application serves as its authenticated role in public.
/// SQL deadlines are owned by its operation contexts; both server timeouts are
/// explicitly disabled. Other applications declare their own schema/role policy.
pub fn low_level_configured(
    options: PgConnectOptions,
    pool: PgPoolOptions,
) -> Result<RunledgerDatabase, sqlx::Error> {
    let profile = session_profile(options.get_username())?;
    runledger_postgres::low_level::connect_database_lazy(options, profile, pool)
}

/// The declared session policy for one authenticated login: it serves as
/// itself, in `public`, with both server timeouts disabled. `configured` uses
/// it for the production pool; tests reuse it for separately provisioned logins.
pub fn session_profile(login: &str) -> Result<PgSessionProfile, sqlx::Error> {
    PgSessionProfile::new(
        login,
        login,
        vec!["public".into()],
        Duration::ZERO,
        Duration::ZERO,
    )
    .map_err(|error| sqlx::Error::Configuration(Box::new(error)))
}

/// Construct and publish closure before exposing the shared application/worker pool.
/// Migration: `configured(options, pool)` now requires a reserved cleanup slot as
/// its first argument. Explicit fixture ownership uses `low_level_configured`.
pub fn configured(
    slot: batter::cleanup::CleanupSlot<'_>,
    options: PgConnectOptions,
    pool: PgPoolOptions,
) -> Result<RunledgerDatabase, sqlx::Error> {
    let profile = session_profile(options.get_username())?;
    let shared = batter::sqlx::pool_in(slot, pool, options, profile)?;
    RunledgerDatabase::from_profiled(shared)
}
