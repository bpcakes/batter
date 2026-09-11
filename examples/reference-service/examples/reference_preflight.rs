//! Read-only live-suite preflight using the same validator and SQLx path as probes.
#[path = "../tests/support/live_endpoint.rs"]
mod live_endpoint;
#[cfg(test)]
#[path = "../tests/support/configuration_process.rs"]
mod process_runtime;

use batter::settings::SettingsError;
use batter_example_reference_service::config::PoolSettings;
use std::{process::ExitCode, time::Duration};

#[tokio::main]
async fn main() -> ExitCode {
    match check().await {
        Ok(()) => {
            println!("reference-preflight:ok");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("reference-preflight: {error}");
            ExitCode::FAILURE
        }
    }
}

// Keep the server predicates from the upstream runner in the native client path.
const PRIMARY_QUERY: &str = "SELECT current_setting('server_version_num')::int / 10000 = 18 \
    AND rolsuper AND current_setting('autovacuum')::boolean \
    AND current_setting('track_counts')::boolean \
    AND current_setting('autovacuum_naptime')::interval <= interval '5 seconds' \
    AND has_table_privilege(current_user, \
    'pg_catalog.pg_shdescription', 'MAINTAIN,UPDATE,DELETE,TRUNCATE'), \
    system_identifier FROM pg_roles CROSS JOIN pg_control_system() \
    WHERE rolname = current_user";
const SECONDARY_QUERY: &str = "SELECT current_setting('server_version_num')::int / 10000 = 18, \
    system_identifier FROM pg_control_system()";

async fn check() -> Result<(), SettingsError> {
    // Validate both endpoints before opening either connection.
    let (_, primary) = live_endpoint::from_process()?;
    let (_, secondary) = live_endpoint::from_named_process("POSTGRES_TEST_OBSERVER_URL")?;
    let primary = inspect(primary, "primary preflight", PRIMARY_QUERY).await?;
    let primary_id = accept_primary(primary)?;
    let secondary = inspect(secondary, "secondary preflight", SECONDARY_QUERY).await?;
    accept_secondary(primary_id, secondary)
}

async fn inspect(
    options: sqlx::postgres::PgConnectOptions,
    field: &'static str,
    query: &'static str,
) -> Result<(bool, i64), SettingsError> {
    let pool = PoolSettings::new(1, 0, Duration::from_secs(10))?
        .pool_options()
        .connect_with(options)
        .await
        .map_err(|error| SettingsError::new(field, "native connection failed").with_cause(error))?;
    // pg_control_system returns a signed bigint, including negative identities.
    let result = tokio::time::timeout(
        Duration::from_secs(10),
        sqlx::query_as::<_, (bool, i64)>(query).fetch_one(&pool),
    )
    .await;
    pool.close().await;
    result
        .map_err(|error| {
            SettingsError::new(field, "prerequisite query timed out").with_cause(error)
        })?
        .map_err(|error| SettingsError::new(field, "prerequisite query failed").with_cause(error))
}

fn accept_primary((accepted, identity): (bool, i64)) -> Result<i64, SettingsError> {
    if !accepted {
        return Err(SettingsError::new(
            "primary preflight",
            "requires PostgreSQL 18, superuser and catalog-lock privileges, autovacuum and track_counts enabled with naptime <= 5s",
        ));
    }
    Ok(identity)
}

fn accept_secondary(
    primary_id: i64,
    (accepted, identity): (bool, i64),
) -> Result<(), SettingsError> {
    if !accepted {
        return Err(SettingsError::new(
            "secondary preflight",
            "requires PostgreSQL 18 and pg_control_system access",
        ));
    }
    if primary_id == identity {
        return Err(SettingsError::new(
            "secondary preflight",
            "endpoints identify the same cluster; select a distinct secondary cluster",
        ));
    }
    Ok(())
}

#[test]
fn child_fixture() {
    if process_runtime::launch::scenario().is_some() {
        assert_eq!(main(), ExitCode::FAILURE);
        std::process::exit(42);
    }
}

#[cfg(test)]
#[path = "reference_preflight/tests.rs"]
mod tests;
