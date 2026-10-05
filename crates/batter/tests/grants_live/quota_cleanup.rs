//! Each quota family's expiry cleanup under a login holding only that selection.
//!
//! Cleanup is independently selectable, so a role that also admits proves
//! nothing about it: admission reads the capacity ledger's `row_count` and
//! cleanup never does, so a cleanup query that acquired that dependency would
//! pass under the combined role and still break a cleanup-only consumer.

use super::policy::{Composition, PublicPolicy};
use super::support::{Fixture, Result, quote, require, require_denied, require_within_policy};
use batter::runlimit::grants::RunlimitOperation;
use batter::runlimit::postgres::{PostgresGcraLimiter, PostgresLimiter};
use sqlx::PgPool;

/// Prove each family's expiry cleanup under a login holding only that selection.
///
/// The owner seeds an already-expired counter, the restricted login's own
/// `cleanup_expired` has to delete it, and the login is refused both the
/// admission insert and the capacity ledger's `row_count` that only admission
/// reads. A role combining admission with cleanup proves none of that.
pub(crate) async fn exercise_cleanup_only(
    fixture: &mut Fixture,
    quotas: &str,
    owner: &PgPool,
) -> Result {
    for family in [Family::FixedWindow, Family::Gcra] {
        let role = Composition::new(PublicPolicy::Deny)
            .with_quotas(quotas, &[family.cleanup()])
            .compile()?;
        let login = fixture.login(family.label(), &role).await?;
        let pool = fixture.pool(&login, &[quotas]).await?;
        require_within_policy(&pool, &role).await?;

        seed_expired_counter(owner, quotas, family, &login).await?;
        let before = super::quota_cases::counter_rows(owner, quotas, family.counters()).await?;
        require(
            before >= 1,
            &format!(
                "the owner could not seed an expired {} counter",
                family.label()
            ),
        )?;
        let deleted = family.cleanup_expired(&pool).await?;
        require(
            deleted >= 1,
            &format!(
                "{} cleanup-only deleted {deleted} rows, expected at least 1",
                family.label()
            ),
        )?;
        require(
            super::quota_cases::counter_rows(owner, quotas, family.counters()).await? == 0,
            &format!(
                "an expired {} counter survived a cleanup-only login",
                family.label()
            ),
        )?;

        for forbidden in [
            format!(
                "SELECT row_count FROM {}.{}",
                quote(quotas),
                family.ledger()
            ),
            format!(
                "INSERT INTO {}.{} DEFAULT VALUES",
                quote(quotas),
                family.counters()
            ),
        ] {
            require_denied(&pool, &forbidden).await?;
        }
    }
    Ok(())
}

/// The two independently selectable quota families, as the cleanup-only proof
/// needs them: their cleanup selection, their counter and capacity relations.
#[derive(Clone, Copy)]
enum Family {
    FixedWindow,
    Gcra,
}

impl Family {
    const fn cleanup(self) -> RunlimitOperation {
        match self {
            Self::FixedWindow => RunlimitOperation::FixedWindowExpiryCleanup,
            Self::Gcra => RunlimitOperation::GcraExpiryCleanup,
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::FixedWindow => "fixed_cleanup",
            Self::Gcra => "gcra_cleanup",
        }
    }

    const fn counters(self) -> &'static str {
        match self {
            Self::FixedWindow => "runlimit_fixed_windows",
            Self::Gcra => "runlimit_gcra",
        }
    }

    const fn ledger(self) -> &'static str {
        match self {
            Self::FixedWindow => "runlimit_capacity_shards",
            Self::Gcra => "runlimit_gcra_shards",
        }
    }

    async fn cleanup_expired(self, pool: &PgPool) -> Result<u64> {
        match self {
            Self::FixedWindow => Ok(PostgresLimiter::new(pool.clone())
                .cleanup_expired(16)
                .await?),
            Self::Gcra => Ok(PostgresGcraLimiter::new(pool.clone())
                .cleanup_expired(16)
                .await?),
        }
    }
}

/// Seed one already-expired counter from the owner. The subject digest is the
/// login name so two cleanup-only roles never share a seeded row.
async fn seed_expired_counter(owner: &PgPool, quotas: &str, family: Family, login: &str) -> Result {
    let quotas = quote(quotas);
    let statement = match family {
        Family::FixedWindow => format!(
            "INSERT INTO {quotas}.runlimit_fixed_windows (policy_id, scope_id, \
                 config_fingerprint, subject_key, window_started_at, \
                 window_expires_at, used) \
             VALUES ('batter.grants.cleanup', 'subject', \
                 sha256('batter.grants.cleanup'), sha256($1::bytea), \
                 now() - interval '2 seconds', now() - interval '1 second', 1)"
        ),
        Family::Gcra => format!(
            "INSERT INTO {quotas}.runlimit_gcra \
                 (config_fingerprint, subject_key, tat_scaled, expires_at_ms) \
             VALUES (sha256('batter.grants.cleanup'), sha256($1::bytea), 0, 0)"
        ),
    };
    sqlx::query(sqlx::AssertSqlSafe(statement))
        .bind(login.as_bytes())
        .execute(owner)
        .await?;
    Ok(())
}
