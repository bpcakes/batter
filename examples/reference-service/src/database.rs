//! Application-owned database policy. Native endpoint validation happens first.
use runledger_postgres::{PgSessionProfile, RunledgerDatabase};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use std::time::Duration;

/// This reference application serves as its authenticated role in public.
/// SQL deadlines are owned by its operation contexts; both server timeouts are
/// explicitly disabled. Other applications declare their own schema/role policy.
pub fn configured(
    options: PgConnectOptions,
    pool: PgPoolOptions,
) -> Result<RunledgerDatabase, sqlx::Error> {
    let login = options.get_username();
    let profile = PgSessionProfile::new(
        login,
        login,
        vec!["public".into()],
        Duration::ZERO,
        Duration::ZERO,
    )
    .map_err(|error| sqlx::Error::Configuration(Box::new(error)))?;
    RunledgerDatabase::connect_lazy(options, profile, pool)
}
