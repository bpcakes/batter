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
        // Expiry is driven by the owner so no admission decision depends on how
        // fast the connection answers.
        let owner = fixture.names.owner.clone();
        let owner_pool = fixture.pool(&owner, &[&quotas]).await?;
        let fixed_login = fixture.login("fixed", &fixed_role).await?;
        let gcra_login = fixture.login("gcra", &gcra_role).await?;
        let fixed_pool = fixture.pool(&fixed_login, &[&quotas]).await?;
        let gcra_pool = fixture.pool(&gcra_login, &[&quotas]).await?;
        require_within_policy(&fixed_pool, &fixed_role).await?;
        require_within_policy(&gcra_pool, &gcra_role).await?;

        // Admission alone is a narrower selection than admission plus cleanup: it
        // needs no relation-level SELECT and no read of the generated
        // `capacity_shard`, so a login that only admits proves the cleanup
        // selection's wider reads are not an admission requirement.
        let admitting_role = Composition::new(PublicPolicy::Deny)
            .with_quotas(&quotas, &[RunlimitOperation::FixedWindowAdmission])
            .compile()?;
        let admitting_login = fixture.login("admitting", &admitting_role).await?;
        let admitting_pool = fixture.pool(&admitting_login, &[&quotas]).await?;
        require_within_policy(&admitting_pool, &admitting_role).await?;
        exercise_fixed_window(&admitting_pool, &quotas, None).await?;
        for forbidden in [
            format!(
                "SELECT capacity_shard FROM {}.runlimit_fixed_windows",
                quote(&quotas)
            ),
            format!("SELECT ctid FROM {}.runlimit_fixed_windows", quote(&quotas)),
            format!("DELETE FROM {}.runlimit_fixed_windows", quote(&quotas)),
        ] {
            require_denied(&admitting_pool, &forbidden).await?;
        }

        // GCRA admission alone, for the same reason as the fixed-window case: a
        // combined role would let the cleanup selection's grants satisfy an
        // admission statement.
        let gcra_admitting_role = Composition::new(PublicPolicy::Deny)
            .with_quotas(&quotas, &[RunlimitOperation::GcraAdmission])
            .compile()?;
        let gcra_admitting_login = fixture
            .login("gcra_admitting", &gcra_admitting_role)
            .await?;
        let gcra_admitting_pool = fixture.pool(&gcra_admitting_login, &[&quotas]).await?;
        require_within_policy(&gcra_admitting_pool, &gcra_admitting_role).await?;
        exercise_gcra(&gcra_admitting_pool, &quotas, None).await?;
        require_denied(
            &gcra_admitting_pool,
            &format!("DELETE FROM {}.runlimit_gcra", quote(&quotas)),
        )
        .await?;

        exercise_fixed_window(&fixed_pool, &quotas, Some(&owner_pool)).await?;
        exercise_gcra(&gcra_pool, &quotas, Some(&owner_pool)).await?;

        require_separated_families(&fixed_pool, &gcra_pool, &quotas).await?;
        Ok(())
    })
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires a dedicated disposable PostgreSQL 18 cluster through BATTER_SQLX_ADMIN_URL"]
#[allow(clippy::too_many_lines)]
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

        // The backoffs are seconds, so a denial window cannot close during a
        // database round trip however slow the connection is, and every wait
        // below uses a settlement's own returned delay or the policy's own quiet
        // period rather than a guessed interval.
        let period = |millis| QuotaPeriod::new(Duration::from_millis(millis));
        let policy = AttemptPolicy::new(
            PolicyId::new("batter.grants.attempts")?,
            ScopeId::new("identifier")?,
            period(1_000)?,
            period(2_000)?,
            period(2_500)?,
            period(5_000)?,
        )?;
        let hasher = hasher();
        let limiter = PostgresAttemptLimiter::new(pool.clone());

        // Admission, failure settlement with an incremented retry delay, and the
        // denial that the resulting quiet period produces.
        let subject = hasher.hash_attempt_for(&policy, "grants-identifier");
        let AttemptAdmission::Admitted(held) = limiter.admit(subject).await? else {
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
        // The first failure's own backoff has to expire before a retry is
        // admissible; waiting for a fixed interval would race the backoff.
        tokio::time::sleep(first.retry_after().duration() + Duration::from_millis(50)).await;
        let AttemptAdmission::Admitted(held) = limiter.admit(subject).await? else {
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
            matches!(limiter.admit(subject).await?, AttemptAdmission::Denied(_)),
            "the second failure's backoff did not deny the next attempt",
        )?;

        // Transactional claim and finish in the caller's own transaction, under a
        // subject whose record the previous backoff does not govern.
        let claimant = hasher.hash_attempt_for(&policy, "grants-transaction");
        let AttemptAdmission::Admitted(held) = limiter.admit(claimant).await? else {
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

        // Success resets the record, so the stored row stays usable. Waiting past
        // the quiet period also makes it eligible for the bounded internal expiry
        // deletion, which is the same login's own DELETE authority.
        require(
            counter_rows(&pool, &quotas, "runlimit_attempts").await? >= 1,
            "no attempt record was stored",
        )?;
        tokio::time::sleep(policy.quiet_period().duration() + Duration::from_millis(100)).await;
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

/// Quota windows long enough that no admission decision depends on how fast the
/// connection answers. Expiry is driven by the owner rewriting the stored
/// timestamp, so the restricted login's own DELETE authority is still what
/// removes the row.
const LONG_WINDOW: Duration = Duration::from_secs(3_600);

/// Fixed-window first insert, counter update, denial and, when the owner expires
/// the stored window, the selected login's real expiry deletion.
async fn exercise_fixed_window(pool: &PgPool, quotas: &str, owner: Option<&PgPool>) -> Result {
    // The subject is unique per login so two selections never share one counter.
    let subject_label: String = sqlx::query_scalar("SELECT gen_random_uuid()::text")
        .fetch_one(pool)
        .await?;
    let policy = FixedWindowPolicy::new(
        PolicyId::new("batter.grants.fixed")?,
        ScopeId::new("subject")?,
        2,
        LONG_WINDOW,
    )?;
    let subject = hasher().hash_for(&policy, subject_label.as_str());
    let limiter = PostgresLimiter::new(pool.clone());
    let before = counter_rows(pool, quotas, "runlimit_fixed_windows").await?;
    // Two admissions then a denial, decided by the stored counter rather than by
    // elapsed time.
    for expected in [true, true, false] {
        let decision = limiter.check(&Check::new(subject)).await?;
        require(
            decision.permits_request() == expected,
            &format!("fixed-window admission returned {decision:?}, expected {expected}"),
        )?;
    }
    require(
        counter_rows(pool, quotas, "runlimit_fixed_windows").await? == before + 1,
        "the fixed-window counter row was not stored",
    )?;
    if let Some(owner) = owner {
        exec_owned(
            owner,
            format!(
                // The stored window keeps its validity check: the start moves
                // back with the expiry so the row is expired, not malformed.
                "UPDATE {}.runlimit_fixed_windows \
                 SET window_started_at = now() - interval '2 seconds', \
                     window_expires_at = now() - interval '1 second'",
                quote(quotas)
            ),
        )
        .await?;
        let deleted = limiter.cleanup_expired(16).await?;
        require(
            deleted >= 1,
            &format!("fixed-window cleanup deleted {deleted} rows, expected at least 1"),
        )?;
        require(
            counter_rows(pool, quotas, "runlimit_fixed_windows").await? == 0,
            "an expired fixed-window counter survived its cleanup",
        )?;
    }

    Ok(())
}

/// GCRA first insert, replenishment denial and, after the owner expires the
/// stored counter, the selected login's own expiry deletion.
async fn exercise_gcra(pool: &PgPool, quotas: &str, owner: Option<&PgPool>) -> Result {
    let gcra_policy = GcraPolicy::new(
        PolicyId::new("batter.grants.gcra")?,
        ScopeId::new("subject")?,
        1,
        LONG_WINDOW,
        1,
    )?;
    // The subject is unique per login so two selections never share one counter.
    let subject_label: String = sqlx::query_scalar("SELECT gen_random_uuid()::text")
        .fetch_one(pool)
        .await?;
    let gcra_subject = hasher().hash_for(&gcra_policy, subject_label.as_str());
    let gcra = PostgresGcraLimiter::new(pool.clone());
    let before = counter_rows(pool, quotas, "runlimit_gcra").await?;
    require(
        gcra.check(&Check::new(gcra_subject))
            .await?
            .permits_request(),
        "the first GCRA request was denied",
    )?;
    // One hour of replenishment is far beyond any connection latency, so the
    // second request is denied by the stored arrival time, not by timing.
    require(
        !gcra
            .check(&Check::new(gcra_subject))
            .await?
            .permits_request(),
        "a second GCRA request inside the replenishment window was admitted",
    )?;
    require(
        counter_rows(pool, quotas, "runlimit_gcra").await? == before + 1,
        "the GCRA counter row was not stored",
    )?;
    let Some(owner) = owner else {
        return Ok(());
    };
    exec_owned(
        owner,
        format!(
            "UPDATE {}.runlimit_gcra SET expires_at_ms = 0",
            quote(quotas)
        ),
    )
    .await?;
    let removed = gcra.cleanup_expired(16).await?;
    require(
        removed >= 1,
        &format!("GCRA cleanup deleted {removed} rows, expected at least 1"),
    )?;
    require(
        counter_rows(pool, quotas, "runlimit_gcra").await? == 0,
        "an expired GCRA counter survived its cleanup",
    )?;
    Ok(())
}

/// Run one owner-driven statement that makes a stored counter expired.
async fn exec_owned(pool: &PgPool, statement: String) -> Result {
    sqlx::raw_sql(sqlx::AssertSqlSafe(statement))
        .execute(pool)
        .await
        .map(|_| ())
        .map_err(Into::into)
}

/// Neither family reaches the other's store, the capacity ledgers' row_count,
/// or the counter-key columns.
async fn require_separated_families(
    fixed_pool: &PgPool,
    gcra_pool: &PgPool,
    quotas: &str,
) -> Result {
    let quotas = quote(quotas);
    for (pool, forbidden) in [
        (fixed_pool, format!("SELECT 1 FROM {quotas}.runlimit_gcra")),
        (
            fixed_pool,
            format!("SELECT 1 FROM {quotas}.runlimit_attempts"),
        ),
        (
            fixed_pool,
            format!("UPDATE {quotas}.runlimit_capacity_shards SET row_count = 0"),
        ),
        (
            fixed_pool,
            format!("UPDATE {quotas}.runlimit_fixed_windows SET config_fingerprint = subject_key"),
        ),
        (
            fixed_pool,
            format!("DELETE FROM {quotas}.runlimit_capacity_shards"),
        ),
        (
            gcra_pool,
            format!("SELECT 1 FROM {quotas}.runlimit_fixed_windows"),
        ),
        (
            gcra_pool,
            format!("UPDATE {quotas}.runlimit_gcra_shards SET row_count = 0"),
        ),
        (
            gcra_pool,
            format!("UPDATE {quotas}.runlimit_gcra SET subject_key = config_fingerprint"),
        ),
    ] {
        require_denied(pool, &forbidden).await?;
    }
    Ok(())
}
