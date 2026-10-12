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
            // No cleanup selection holds relation-level SELECT, which a system
            // column read would need and which would expose every column.
            format!("SELECT ctid FROM {}.{}", quote(quotas), family.counters()),
        ] {
            require_denied(&pool, &forbidden).await?;
        }
        for column in family.unread_counter_columns() {
            require_denied(
                &pool,
                &format!(
                    "SELECT {column} FROM {}.{}",
                    quote(quotas),
                    family.counters()
                ),
            )
            .await?;
        }
    }
    Ok(())
}

/// The two independently selectable quota families, as the cleanup proofs need
/// them: their selections, their counter and capacity relations.
#[derive(Clone, Copy)]
pub(crate) enum Family {
    FixedWindow,
    Gcra,
}

impl Family {
    pub(crate) const fn admission(self) -> RunlimitOperation {
        match self {
            Self::FixedWindow => RunlimitOperation::FixedWindowAdmission,
            Self::Gcra => RunlimitOperation::GcraAdmission,
        }
    }

    pub(crate) const fn cleanup(self) -> RunlimitOperation {
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

    pub(crate) const fn counters(self) -> &'static str {
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

    /// Counter columns that no cleanup statement reads.
    const fn unread_counter_columns(self) -> &'static [&'static str] {
        match self {
            Self::FixedWindow => &["used", "policy_id", "scope_id", "window_started_at"],
            Self::Gcra => &["tat_scaled"],
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

/// Seed one already-expired counter from the owner. The subject digest is
/// derived from `subject`, which callers keep unique so no two seeds share a
/// row.
pub(crate) async fn seed_expired_counter(
    owner: &PgPool,
    quotas: &str,
    family: Family,
    subject: &str,
) -> Result {
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
        .bind(subject.as_bytes())
        .execute(owner)
        .await?;
    Ok(())
}

/// How many counters seeded for `subject` remain.
pub(crate) async fn seeded_rows(
    owner: &PgPool,
    quotas: &str,
    family: Family,
    subject: &str,
) -> Result<i64> {
    Ok(sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT count(*) FROM {}.{} WHERE subject_key = sha256($1::bytea)",
        quote(quotas),
        family.counters()
    )))
    .bind(subject.as_bytes())
    .fetch_one(owner)
    .await?)
}
