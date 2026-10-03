//! Native Runlimit storage operations under their selected grants.

use super::policy::{Composition, PublicPolicy};
use super::schema;
use super::support::{Fixture, Result, quote, require, require_denied, require_within_policy};
use batter::runlimit::grants::RunlimitOperation;
use batter::runlimit::native::attempts::{
    AttemptAdmission, AttemptCompletionResult, AttemptOutcome, AttemptPolicy,
    StagedAttemptCompletion,
};
use batter::runlimit::native::{
    Check, FixedWindowPolicy, GcraPolicy, KeyHasher, PolicyId, QuotaPeriod, ScopeId,
};
use batter::runlimit::postgres::attempts::{
    PgAttemptClaimResult, PostgresAttemptLimiter, low_level,
};
use batter::runlimit::postgres::{PostgresGcraLimiter, PostgresLimiter};
use sqlx::PgPool;
use std::time::Duration;

fn hasher() -> KeyHasher {
    KeyHasher::new([19; 32]).expect("a 32-byte key is valid")
}

async fn counter_rows(pool: &PgPool, schema: &str, relation: &str) -> Result<i64> {
    Ok(sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT count(*) FROM {}.{relation}",
        quote(schema)
    )))
    .fetch_one(pool)
    .await?)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires a dedicated disposable PostgreSQL 18 cluster through BATTER_SQLX_ADMIN_URL"]
async fn fixed_window_and_gcra_families_admit_deny_and_delete_only_their_own_stores() -> Result {
    Fixture::run(async |fixture| {
        schema::install(fixture).await?;
        let quotas = fixture.names.quotas.clone();

        let fixed_role = Composition::new(PublicPolicy::Deny)
            .with_quotas(
                &quotas,
                &[
                    RunlimitOperation::FixedWindowAdmission,
                    RunlimitOperation::FixedWindowExpiryCleanup,
                ],
            )
            .compile()?;
        let gcra_role = Composition::new(PublicPolicy::Deny)
            .with_quotas(
                &quotas,
                &[
                    RunlimitOperation::GcraAdmission,
                    RunlimitOperation::GcraExpiryCleanup,
                ],
            )
            .compile()?;
        let fixed_login = fixture.login("fixed", &fixed_role).await?;
        let gcra_login = fixture.login("gcra", &gcra_role).await?;
        let fixed_pool = fixture.pool(&fixed_login, &[&quotas]).await?;
        let gcra_pool = fixture.pool(&gcra_login, &[&quotas]).await?;
        require_within_policy(&fixed_pool, &fixed_role).await?;
        require_within_policy(&gcra_pool, &gcra_role).await?;

        // Fixed-window: first insert, counter update, denial, then real expiry
        // deletion through the capacity ledger's definer triggers.
        let policy = FixedWindowPolicy::new(
            PolicyId::new("batter.grants.fixed")?,
            ScopeId::new("subject")?,
            2,
            Duration::from_millis(400),
        )?;
        let subject = hasher().hash_for(&policy, "grants-subject");
        let limiter = PostgresLimiter::new(fixed_pool.clone());
        for expected in [true, true, false] {
            let decision = limiter.check(&Check::new(subject.clone())).await?;
            require(
                decision.permits_request() == expected,
                &format!("fixed-window admission returned {decision:?}, expected {expected}"),
            )?;
        }
        require(
            counter_rows(&fixed_pool, &quotas, "runlimit_fixed_windows").await? == 1,
            "the fixed-window counter row was not stored",
        )?;
        tokio::time::sleep(Duration::from_millis(500)).await;
        let deleted = limiter.cleanup_expired(16).await?;
        require(
            deleted == 1,
            &format!("fixed-window cleanup deleted {deleted} rows, expected 1"),
        )?;

        // GCRA: first insert, replenishment denial and its own expiry deletion.
        let gcra_policy = GcraPolicy::new(
            PolicyId::new("batter.grants.gcra")?,
            ScopeId::new("subject")?,
            1,
            Duration::from_millis(400),
            1,
        )?;
        let gcra_subject = hasher().hash_for(&gcra_policy, "grants-subject");
        let gcra = PostgresGcraLimiter::new(gcra_pool.clone());
        require(
            gcra.check(&Check::new(gcra_subject.clone()))
                .await?
                .permits_request(),
            "the first GCRA request was denied",
        )?;
        require(
            !gcra
                .check(&Check::new(gcra_subject.clone()))
                .await?
                .permits_request(),
            "an immediate second GCRA request was admitted",
        )?;
        require(
            counter_rows(&gcra_pool, &quotas, "runlimit_gcra").await? == 1,
            "the GCRA counter row was not stored",
        )?;
        tokio::time::sleep(Duration::from_millis(500)).await;
        let removed = gcra.cleanup_expired(16).await?;
        require(
            removed == 1,
            &format!("GCRA cleanup deleted {removed} rows, expected 1"),
        )?;

        // Neither family reaches the other's store, the capacity ledgers'
        // row_count, or the counter-key columns.
        for (pool, forbidden) in [
            (
                &fixed_pool,
                format!("SELECT 1 FROM {}.runlimit_gcra", quote(&quotas)),
            ),
            (
                &fixed_pool,
                format!("SELECT 1 FROM {}.runlimit_attempts", quote(&quotas)),
            ),
            (
                &fixed_pool,
                format!(
                    "UPDATE {}.runlimit_capacity_shards SET row_count = 0",
                    quote(&quotas)
                ),
            ),
            (
                &fixed_pool,
                format!(
                    "UPDATE {}.runlimit_fixed_windows SET config_fingerprint = subject_key",
                    quote(&quotas)
                ),
            ),
            (
                &fixed_pool,
                format!("DELETE FROM {}.runlimit_capacity_shards", quote(&quotas)),
            ),
            (
                &gcra_pool,
                format!("SELECT 1 FROM {}.runlimit_fixed_windows", quote(&quotas)),
            ),
            (
                &gcra_pool,
                format!(
                    "UPDATE {}.runlimit_gcra_shards SET row_count = 0",
                    quote(&quotas)
                ),
            ),
            (
                &gcra_pool,
                format!(
                    "UPDATE {}.runlimit_gcra SET subject_key = config_fingerprint",
                    quote(&quotas)
                ),
            ),
        ] {
            require_denied(pool, &forbidden).await?;
        }
        Ok(())
    })
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires a dedicated disposable PostgreSQL 18 cluster through BATTER_SQLX_ADMIN_URL"]
async fn authentication_attempts_admit_settle_and_clean_up_under_one_selection() -> Result {
    Fixture::run(async |fixture| {
        schema::install(fixture).await?;
        let quotas = fixture.names.quotas.clone();
        let role = Composition::new(PublicPolicy::Deny)
            .with_quotas(&quotas, &[RunlimitOperation::AuthenticationAttempts])
            .compile()?;
        let login = fixture.login("attempts", &role).await?;
        let pool = fixture.pool(&login, &[&quotas]).await?;
        require_within_policy(&pool, &role).await?;

        let period = |millis| QuotaPeriod::new(Duration::from_millis(millis));
        let policy = AttemptPolicy::new(
            PolicyId::new("batter.grants.attempts")?,
            ScopeId::new("identifier")?,
            period(5)?,
            period(100)?,
            period(300)?,
            period(1_000)?,
        )?;
        let hasher = hasher();
        let limiter = PostgresAttemptLimiter::new(pool.clone());

        // Admission, failure settlement with an incremented retry delay, and the
        // denial that the resulting quiet period produces.
        let subject = hasher.hash_attempt_for(&policy, "grants-identifier");
        let AttemptAdmission::Admitted(held) = limiter.admit(subject.clone()).await? else {
            return Err(super::support::fail("the first attempt was not admitted"));
        };
        let AttemptCompletionResult::Applied(first) =
            limiter.complete(held, AttemptOutcome::Failure).await?
        else {
            return Err(super::support::fail(
                "the first settlement was reported stale",
            ));
        };
        require(
            first.consecutive_failures() == 1,
            &format!("failure settlement reported {first:?}"),
        )?;
        let AttemptAdmission::Admitted(held) = limiter.admit(subject.clone()).await? else {
            return Err(super::support::fail("the retry was not admitted"));
        };
        let AttemptCompletionResult::Applied(second) =
            limiter.complete(held, AttemptOutcome::Failure).await?
        else {
            return Err(super::support::fail(
                "the retry settlement was reported stale",
            ));
        };
        require(
            second.consecutive_failures() == 2 && second.retry_after() > first.retry_after(),
            &format!("the retry delay did not increase: {first:?} then {second:?}"),
        )?;
        require(
            matches!(
                limiter.admit(subject.clone()).await?,
                AttemptAdmission::Denied(_)
            ),
            "the quiet period did not deny the next attempt",
        )?;

        // Transactional claim and finish in the caller's own transaction.
        tokio::time::sleep(Duration::from_millis(400)).await;
        let claimant = hasher.hash_attempt_for(&policy, "grants-transaction");
        let AttemptAdmission::Admitted(held) = limiter.admit(claimant.clone()).await? else {
            return Err(super::support::fail(
                "the transactional attempt was not admitted",
            ));
        };
        let mut transaction = pool.begin().await?;
        let PgAttemptClaimResult::Claimed(claimed) =
            low_level::claim_in(&mut *transaction, held).await?
        else {
            return Err(super::support::fail(
                "the live reservation could not be claimed",
            ));
        };
        let StagedAttemptCompletion::Applied(settled) =
            low_level::finish_in(&mut *transaction, claimed, AttemptOutcome::Success).await?
        else {
            return Err(super::support::fail(
                "the staged settlement was reported stale",
            ));
        };
        transaction.commit().await?;
        require(
            settled.consecutive_failures() == 0,
            &format!("success settlement reported {settled:?}"),
        )?;

        // Success resets the record, so the stored row stays usable. Internal
        // expiry cleanup is the same login's own DELETE authority.
        require(
            counter_rows(&pool, &quotas, "runlimit_attempts").await? >= 1,
            "no attempt record was stored",
        )?;
        tokio::time::sleep(Duration::from_millis(500)).await;
        let AttemptAdmission::Admitted(held) = limiter.admit(claimant).await? else {
            return Err(super::support::fail(
                "a settled subject could not be readmitted",
            ));
        };
        let _ = limiter.complete(held, AttemptOutcome::Success).await?;

        for forbidden in [
            format!("SELECT 1 FROM {}.runlimit_fixed_windows", quote(&quotas)),
            format!("SELECT 1 FROM {}.runlimit_gcra", quote(&quotas)),
            format!(
                "UPDATE {}.runlimit_attempts SET capacity_slot = 0",
                quote(&quotas)
            ),
            format!(
                "UPDATE {}.runlimit_attempts SET subject_key = config_fingerprint",
                quote(&quotas)
            ),
        ] {
            require_denied(&pool, &forbidden).await?;
        }
        Ok(())
    })
    .await
}
