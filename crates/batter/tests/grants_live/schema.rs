//! Owner-applied native migrations and the application's own objects.

use super::policy::{APPLICATION_RELATION, APPLICATION_ROUTINE};
use super::support::{Fixture, Result, exec, quote};
use batter::runledger::{PgSessionProfile, RunledgerDatabase};
use batter::runlimit::postgres::attempts::PostgresAttemptLimiter;
use batter::runlimit::postgres::{PostgresGcraLimiter, PostgresLimiter};
use sqlx::PgPool;
use std::time::Duration;

/// Apply the native schemas as their owner and create the application objects.
///
/// Runledger and Runlimit keep separate schemas, so each owns its own
/// `_sqlx_migrations` history and neither migrator has to tolerate the other's
/// versions.
pub(crate) async fn install(fixture: &mut Fixture) -> Result {
    let owner = fixture.names.owner.clone();
    let jobs = fixture.names.jobs.clone();
    let quotas = fixture.names.quotas.clone();
    let application = fixture.names.application.clone();
    let shadow = fixture.names.shadow.clone();

    let database = runledger_database(fixture, &owner, &jobs, 2)?;
    batter::runledger::native::postgres::migrate_after_idempotency_cutover(&database).await?;
    fixture.track(database.pool().clone());

    let quota_pool = fixture.pool(&owner, &[&quotas]).await?;
    PostgresLimiter::new(quota_pool.clone()).migrate().await?;
    PostgresGcraLimiter::new(quota_pool.clone())
        .migrate()
        .await?;
    PostgresAttemptLimiter::new(quota_pool).migrate().await?;

    let owner_pool = fixture.pool(&owner, &[&application]).await?;
    let mut owned = owner_pool.acquire().await?;
    for statement in [
        format!(
            "CREATE TABLE {}.{} (id uuid PRIMARY KEY DEFAULT gen_random_uuid(), note text NOT NULL)",
            quote(&application),
            quote(APPLICATION_RELATION)
        ),
        format!(
            "CREATE FUNCTION {}.{}(value text) RETURNS text LANGUAGE sql IMMUTABLE \
             AS $$ SELECT lower(value) $$",
            quote(&application),
            quote(APPLICATION_ROUTINE)
        ),
        // Same-named relations in an out-of-scope schema. Grants here must never
        // satisfy a requirement on the selected native schema.
        format!(
            "CREATE TABLE {}.job_enqueue_intents (id uuid PRIMARY KEY, enqueue_request jsonb)",
            quote(&shadow)
        ),
        format!(
            "CREATE TABLE {}.runlimit_gcra (config_fingerprint bytea, subject_key bytea)",
            quote(&shadow)
        ),
    ] {
        exec(&mut owned, statement).await?;
    }
    drop(owned);

    // The native trigger functions stay trusted and installed while serving
    // logins and PUBLIC hold no EXECUTE on them. Application routines follow the
    // application's own PUBLIC policy, declared per composition.
    for schema in [&jobs, &quotas, &application] {
        exec(
            &mut fixture.admin,
            format!(
                "REVOKE ALL ON ALL ROUTINES IN SCHEMA {} FROM PUBLIC",
                quote(schema)
            ),
        )
        .await?;
    }
    Ok(())
}

/// A profiled Runledger database for `role` over the authoritative jobs schema.
pub(crate) fn runledger_database(
    fixture: &Fixture,
    role: &str,
    schema: &str,
    connections: u32,
) -> Result<RunledgerDatabase> {
    let profile = PgSessionProfile::new(
        role,
        role,
        vec![schema.to_owned()],
        Duration::ZERO,
        Duration::ZERO,
    )?;
    Ok(RunledgerDatabase::connect_lazy(
        fixture.options(role)?,
        profile,
        sqlx::postgres::PgPoolOptions::new()
            .max_connections(connections)
            .min_connections(0)
            .acquire_timeout(Duration::from_secs(20)),
    )?)
}

/// Grant the application's explicitly permitted PUBLIC delivery.
pub(crate) async fn permit_public_application_delivery(fixture: &mut Fixture) -> Result {
    let application = fixture.names.application.clone();
    for statement in [
        format!(
            "GRANT SELECT (id, note) ON {}.{} TO PUBLIC",
            quote(&application),
            quote(APPLICATION_RELATION)
        ),
        format!(
            "GRANT EXECUTE ON FUNCTION {}.{}(text) TO PUBLIC",
            quote(&application),
            quote(APPLICATION_ROUTINE)
        ),
    ] {
        exec(&mut fixture.admin, statement).await?;
    }
    Ok(())
}

/// Count the rows a privileged owner connection can see, for oracle assertions.
pub(crate) async fn scalar<T>(pool: &PgPool, sql: String) -> Result<T>
where
    T: for<'a> sqlx::Decode<'a, sqlx::Postgres> + sqlx::Type<sqlx::Postgres> + Send + Unpin,
{
    Ok(sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
        .fetch_one(pool)
        .await?)
}
