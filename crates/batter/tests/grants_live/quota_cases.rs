//! Native Runlimit storage operations under their selected grants.

use super::policy::{Composition, PublicPolicy};
use super::schema;
use super::support::{Fixture, Result, quote, require, require_denied, require_within_policy};
use batter::runlimit::grants::RunlimitOperation;
use batter::runlimit::native::attempts::{
    AttemptAdmission, AttemptCompletionResult, AttemptOutcome, AttemptPolicy, AttemptSubject,
    StagedAttemptCompletion,
};
use batter::runlimit::native::{
    BatchDecisionView, Check, FixedWindowPolicy, GcraPolicy, KeyHasher, PolicyId, QuotaPeriod,
    ScopeId,
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

pub(crate) async fn counter_rows(pool: &PgPool, schema: &str, relation: &str) -> Result<i64> {
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

        // Cleanup is independently selectable, so it needs its own login too.
        // Under the combined roles above an admission grant can satisfy a
        // cleanup statement: admission reads the capacity ledger's row_count and
        // cleanup never does, so a cleanup query that acquired that dependency
        // would pass and still break a cleanup-only consumer.
        super::quota_cleanup::exercise_cleanup_only(fixture, &quotas, &owner_pool).await?;

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
        // Leases are expired and expired neighbours planted by the owner, so no
        // assertion below depends on how fast the connection answers.
        let owner = fixture.names.owner.clone();
        let owner_pool = fixture.pool(&owner, &[&quotas]).await?;

        // Every backoff, quiet period and lease is hours long, so none of them can
        // close during the test however slow the connection is. Eligibility is
        // produced by the owner rewriting the stored timestamps, never by
        // waiting, so no assertion below depends on elapsed wall-clock time.
        let period = |millis| QuotaPeriod::new(Duration::from_millis(millis));
        let policy = AttemptPolicy::new(
            PolicyId::new("batter.grants.attempts")?,
            ScopeId::new("identifier")?,
            period(3_600_000)?,
            period(7_200_000)?,
            period(7_200_000)?,
            period(3_600_000)?,
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
        // The first failure's backoff has to be over before a retry is
        // admissible. The owner retires it rather than the test waiting out an
        // hour, and the quiet period is untouched, so the retained failure count
        // is what the next admission carries forward.
        clear_backoff(&owner_pool, &quotas, subject).await?;
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
        // A record the owner has placed outside its quiet period no longer
        // governs the next attempt: the admission admits despite the stored
        // backoff and starts the failure count again, whether the login's own
        // bounded cleanup removed the record first or the admission reset it.
        retire_quiet_period(&owner_pool, &quotas, subject).await?;
        let AttemptAdmission::Admitted(held) = limiter.admit(subject).await? else {
            return Err(super::support::fail(
                "a record outside its quiet period still denied the next attempt",
            ));
        };
        let AttemptCompletionResult::Applied(reset) =
            limiter.complete(held, AttemptOutcome::Failure).await?
        else {
            return Err(super::support::fail(
                "the settlement after the quiet period was reported stale",
            ));
        };
        require(
            reset.consecutive_failures() == 1,
            &format!("the quiet period did not clear the failure history: {reset:?}"),
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

        // A completion whose lease no longer holds is refused without changing
        // the record. The owner expires the lease rather than the test waiting
        // for it, so the refusal is the fencing check and never a slow clock.
        let stale = hasher.hash_attempt_for(&policy, "grants-stale");
        let AttemptAdmission::Admitted(held) = limiter.admit(stale).await? else {
            return Err(super::support::fail(
                "the stale-completion subject was not admitted",
            ));
        };
        expire_lease(&owner_pool, &quotas, stale).await?;
        require(
            matches!(
                limiter.complete(held, AttemptOutcome::Failure).await?,
                AttemptCompletionResult::Stale
            ),
            "completing an expired lease was applied instead of refused",
        )?;

        // A claim that escapes its own transaction is refused the same way: the
        // rollback reverts the claim's private token rotation, so finishing it
        // outside that transaction stages nothing.
        let escaped = hasher.hash_attempt_for(&policy, "grants-escaped-claim");
        let AttemptAdmission::Admitted(held) = limiter.admit(escaped).await? else {
            return Err(super::support::fail(
                "the escaped-claim subject was not admitted",
            ));
        };
        let mut transaction = pool.begin().await?;
        let PgAttemptClaimResult::Claimed(claimed) =
            low_level::claim_in(&mut *transaction, held).await?
        else {
            return Err(super::support::fail(
                "the escaped-claim reservation could not be claimed",
            ));
        };
        transaction.rollback().await?;
        require(
            matches!(
                low_level::finish_in(&pool, claimed, AttemptOutcome::Failure).await?,
                StagedAttemptCompletion::Stale
            ),
            "a claim finished outside its transaction was staged instead of refused",
        )?;

        // The bounded internal expiry runs for the admitted subject's capacity
        // shard on every admission. The owner plants an already-expired row in
        // that same shard, which the login's own DELETE authority then removes;
        // readmission alone would not distinguish deletion from an update.
        let neighbour = hasher.hash_attempt_for(&policy, "grants-expiry");
        let AttemptAdmission::Admitted(held) = limiter.admit(neighbour).await? else {
            return Err(super::support::fail("the expiry subject was not admitted"));
        };
        let _ = limiter.complete(held, AttemptOutcome::Success).await?;
        plant_expired_neighbour(&owner_pool, &quotas, neighbour).await?;
        require(
            planted_rows(&owner_pool, &quotas).await? == 1,
            "the owner could not plant an expired attempt record",
        )?;
        let AttemptAdmission::Admitted(held) = limiter.admit(neighbour).await? else {
            return Err(super::support::fail(
                "the expiry subject was not readmitted",
            ));
        };
        let _ = limiter.complete(held, AttemptOutcome::Success).await?;
        require(
            planted_rows(&owner_pool, &quotas).await? == 0,
            "the expired neighbouring attempt record survived the admission's cleanup",
        )?;

        // Success resets the record rather than removing it, so the stored row
        // stays inside its quiet period and the same login readmits the subject
        // through the record it already has.
        require(
            counter_rows(&pool, &quotas, "runlimit_attempts").await? >= 1,
            "no attempt record was stored",
        )?;
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

/// A capacity slot the admission protocol never assigns on its own, reserved for
/// the owner-planted expired record so it is identifiable and cannot collide.
const PLANTED_SLOT: i32 = 65_535;

/// Retire a stored backoff from the owner so a retry is admissible at once,
/// leaving `last_failure_ms` alone so the quiet period still has hours to run
/// and the retained failure count carries into the next admission.
async fn clear_backoff(owner: &PgPool, quotas: &str, subject: AttemptSubject<'_>) -> Result {
    rewrite_record(owner, quotas, subject, "retry_at_ms = 0").await
}

/// Place a settled record's last failure outside its quiet period from the
/// owner, so readmission follows the stored record rather than elapsed time.
async fn retire_quiet_period(owner: &PgPool, quotas: &str, subject: AttemptSubject<'_>) -> Result {
    rewrite_record(
        owner,
        quotas,
        subject,
        "last_failure_ms = 0, retry_at_ms = 0",
    )
    .await
}

/// Expire a held lease from the owner so a completion has to report staleness.
async fn expire_lease(owner: &PgPool, quotas: &str, subject: AttemptSubject<'_>) -> Result {
    rewrite_record(owner, quotas, subject, "lease_until_ms = 0").await
}

/// Apply one owner-written assignment to exactly one subject's attempt record.
async fn rewrite_record(
    owner: &PgPool,
    quotas: &str,
    subject: AttemptSubject<'_>,
    assignment: &str,
) -> Result {
    let key = subject.into_unbound_subject_key().into_bytes();
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "UPDATE {}.runlimit_attempts SET {assignment} WHERE subject_key = $1",
        quote(quotas)
    )))
    .bind(key.as_slice())
    .execute(owner)
    .await?;
    Ok(())
}

/// Plant an already-expired record that shares the subject's capacity shard.
///
/// `capacity_shard` is generated from the first byte of the fingerprint and of
/// the subject key, so changing only the last byte keeps the planted record in
/// the shard the next admission cleans up. A one-millisecond quiet period with a
/// zero last failure makes it expired against any server clock.
async fn plant_expired_neighbour(
    owner: &PgPool,
    quotas: &str,
    subject: AttemptSubject<'_>,
) -> Result {
    let key = subject.into_unbound_subject_key().into_bytes();
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "INSERT INTO {quotas}.runlimit_attempts \
         (config_fingerprint, subject_key, capacity_slot, failures, \
          last_failure_ms, retry_at_ms, quiet_ms) \
         SELECT config_fingerprint, \
             set_byte(subject_key, 31, (get_byte(subject_key, 31) + 1) % 256), \
             $2, 0, 0, 0, 1 \
         FROM {quotas}.runlimit_attempts WHERE subject_key = $1",
        quotas = quote(quotas)
    )))
    .bind(key.as_slice())
    .bind(PLANTED_SLOT)
    .execute(owner)
    .await?;
    Ok(())
}

/// Count the owner-planted expired records, identified by their quiet period.
async fn planted_rows(owner: &PgPool, quotas: &str) -> Result<i64> {
    Ok(sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT count(*) FROM {}.runlimit_attempts WHERE quiet_ms = 1",
        quote(quotas)
    )))
    .fetch_one(owner)
    .await?)
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
    // A batch admission takes the multi-item row-lock, capacity-lock and upsert
    // path, which a single check never reaches.
    let batch_subjects = [
        hasher().hash_for(&policy, format!("{subject_label}-batch-a").as_str()),
        hasher().hash_for(&policy, format!("{subject_label}-batch-b").as_str()),
    ];
    let batch = limiter.check_all(&batch_subjects.map(Check::new)).await?;
    require(
        matches!(batch.view(), BatchDecisionView::Allowed { allowances } if allowances.len() == 2),
        &format!("the fixed-window batch admission was not allowed: {batch:?}"),
    )?;
    require(
        counter_rows(pool, quotas, "runlimit_fixed_windows").await? == before + 3,
        "the fixed-window counter rows were not stored",
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
    // Two admissions then a denial: the second request has to be allowed so the
    // upsert's existing-counter UPDATE path executes, not only its insert.
    let gcra_policy = GcraPolicy::new(
        PolicyId::new("batter.grants.gcra")?,
        ScopeId::new("subject")?,
        2,
        LONG_WINDOW,
        2,
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
    require(
        gcra.check(&Check::new(gcra_subject))
            .await?
            .permits_request(),
        "the second GCRA request did not update the stored counter",
    )?;
    // One hour of replenishment is far beyond any connection latency, so the
    // third request is denied by the stored arrival time, not by timing.
    require(
        !gcra
            .check(&Check::new(gcra_subject))
            .await?
            .permits_request(),
        "a third GCRA request inside the replenishment window was admitted",
    )?;
    // A batch admission takes the multi-item locking and upsert path, which a
    // single check never reaches.
    let batch_subjects = [
        hasher().hash_for(&gcra_policy, format!("{subject_label}-batch-a").as_str()),
        hasher().hash_for(&gcra_policy, format!("{subject_label}-batch-b").as_str()),
    ];
    let batch = gcra.check_all(&batch_subjects.map(Check::new)).await?;
    require(
        matches!(batch.view(), BatchDecisionView::Allowed { allowances } if allowances.len() == 2),
        &format!("the GCRA batch admission was not allowed: {batch:?}"),
    )?;
    require(
        counter_rows(pool, quotas, "runlimit_gcra").await? == before + 3,
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
