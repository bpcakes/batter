//! Supervised PostgreSQL limiters whose registered expiry cleanup runs under
//! the documented least-privilege login.
//!
//! Each maintained limiter serves and cleans on one pool whose login holds its
//! store's admission and expiry-cleanup selections, and nothing else. The pool
//! is owned by a cleanup finalizer, exactly as an application owns it, so the
//! cases also observe that drain has stopped maintenance before it closes.

use super::policy::{Composition, PublicPolicy};
use super::quota_cleanup::{Family, seed_expired_counter, seeded_rows};
use super::schema;
use super::support::{
    DENIED, Fixture, Result, both, fail, quote, require, require_denied, require_within_policy,
};
use batter::cleanup::CleanupBudget;
use batter::lifecycle::{RunningSupervisor, ShutdownBudget, Supervisor};
use batter::periodic::{PeriodicCompletion, PeriodicReader, PeriodicShutdown, PeriodicStartup};
use batter::runlimit::maintenance::{
    ExpiringStore, ExpiryMaintenance, MaintainedLimiter, register_in,
};
use batter::runlimit::native::{
    Check, FixedWindowPolicy, GcraPolicy, KeyHasher, Limiter, PolicyId, ScopeId, SubjectKey,
};
use batter::runlimit::postgres::{MaintenanceError, PostgresGcraLimiter, PostgresLimiter};
use sqlx::{
    PgPool,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use std::{
    num::NonZeroU32,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::time::{Instant, sleep, timeout};

const INTERVAL: Duration = Duration::from_millis(100);
const ROWS_PER_RUN: NonZeroU32 = NonZeroU32::new(16).unwrap();
const PATIENCE: Duration = Duration::from_secs(60);

fn supervisor() -> Result<Supervisor> {
    let ten = Duration::from_secs(10);
    Ok(Supervisor::new(ShutdownBudget::new(
        ten,
        ten,
        ten,
        CleanupBudget::new(ten, ten, ten)?,
    )?))
}

/// Connection options for `login` with the quota schema as its only search path.
fn quota_options(fixture: &Fixture, login: &str, quotas: &str) -> Result<PgConnectOptions> {
    let search_path = quote(quotas);
    Ok(fixture
        .options(login)?
        .options([("search_path", search_path.as_str())]))
}

/// One family's native limiter: its construction and one admission.
trait Store: ExpiringStore + Sized {
    fn on(pool: PgPool) -> Self;

    /// Admit a new subject through the maintained limiter and return the key
    /// its counter is stored under.
    async fn admit(limiter: &MaintainedLimiter<Self>, label: &str) -> Result<SubjectKey>;
}

fn hasher() -> Result<KeyHasher> {
    Ok(KeyHasher::new([29; 32])?)
}

impl Store for PostgresLimiter {
    fn on(pool: PgPool) -> Self {
        Self::new(pool)
    }

    async fn admit(limiter: &MaintainedLimiter<Self>, label: &str) -> Result<SubjectKey> {
        let policy = FixedWindowPolicy::new(
            PolicyId::new("batter.grants.maintained")?,
            ScopeId::new("subject")?,
            1,
            Duration::from_secs(3_600),
        )?;
        let subject = hasher()?.hash_for(&policy, label);
        let decision = limiter.check(&Check::new(subject)).await?;
        require!(decision.permits_request(), "the maintained limiter denied")?;
        Ok(subject.into_unbound_subject_key())
    }
}

impl Store for PostgresGcraLimiter {
    fn on(pool: PgPool) -> Self {
        Self::new(pool)
    }

    async fn admit(limiter: &MaintainedLimiter<Self>, label: &str) -> Result<SubjectKey> {
        let policy = GcraPolicy::new(
            PolicyId::new("batter.grants.maintained")?,
            ScopeId::new("subject")?,
            1,
            Duration::from_secs(3_600),
            1,
        )?;
        let subject = hasher()?.hash_for(&policy, label);
        let decision = limiter.check(&Check::new(subject)).await?;
        require!(decision.permits_request(), "the maintained limiter denied")?;
        Ok(subject.into_unbound_subject_key())
    }
}

/// Render the first retained cleanup failure with its native cause.
fn retained_cause(reader: &PeriodicReader) -> String {
    let Some(sample) = reader.snapshot().first_failure else {
        return "none".to_owned();
    };
    let mut rendered = sample.error.to_string();
    let mut cause = sample.error.source();
    while let Some(current) = cause {
        // Some native errors already end with their cause's text.
        let text = current.to_string();
        if !rendered.ends_with(&text) {
            rendered.push_str(": ");
            rendered.push_str(&text);
        }
        cause = current.source();
    }
    rendered
}

/// Wait until `condition` holds, failing with `what` after a bounded wait.
async fn eventually<F>(what: &str, mut condition: F) -> Result
where
    F: AsyncFnMut() -> Result<bool>,
{
    let deadline = Instant::now() + PATIENCE;
    while !condition().await? {
        require!(Instant::now() < deadline, what)?;
        sleep(Duration::from_millis(25)).await;
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires a dedicated disposable PostgreSQL 18 cluster through BATTER_SQLX_ADMIN_URL"]
async fn maintained_limiters_clean_expired_rows_under_their_selections_until_drain() -> Result {
    Fixture::run(async |fixture| {
        schema::install(fixture).await?;
        let quotas = fixture.names.quotas.clone();
        let owner = fixture.names.owner.clone();
        let owner_pool = fixture.pool(&owner, &[&quotas]).await?;
        maintain::<PostgresLimiter>(fixture, &owner_pool, Family::FixedWindow).await?;
        maintain::<PostgresGcraLimiter>(fixture, &owner_pool, Family::Gcra).await
    })
    .await
}

/// Run one family's maintained limiter through readiness, recurring cleanup,
/// admission and drain under its documented login.
async fn maintain<L: Store>(fixture: &mut Fixture, owner: &PgPool, family: Family) -> Result {
    let quotas = fixture.names.quotas.clone();
    let role = Composition::new(PublicPolicy::Deny)
        .with_quotas(&quotas, &[family.admission(), family.cleanup()])
        .compile()?;
    let label = match family {
        Family::FixedWindow => "fixed_maintained",
        Family::Gcra => "gcra_maintained",
    };
    let login = fixture.login(label, &role).await?;
    // The login holds exactly its documented selections, and neither carries
    // the relation-level SELECT a system-column read would need.
    let inspection = fixture.pool(&login, &[&quotas]).await?;
    require_within_policy(&inspection, &role).await?;
    require_denied(
        &inspection,
        &format!("SELECT ctid FROM {}.{}", quote(&quotas), family.counters()),
    )
    .await?;

    let before_start = format!("{login}-before-start");
    seed_expired_counter(owner, &quotas, family, &before_start).await?;

    let mut supervisor = supervisor()?;
    let pool = batter::sqlx::pool_in(
        supervisor.reserve_cleanup("quota.pool")?,
        PgPoolOptions::new().max_connections(2),
        quota_options(fixture, &login, &quotas)?,
    );
    fixture.track(pool.clone());
    let limiter = register_in(
        &mut supervisor,
        "quota.maintenance",
        L::on(pool.clone()),
        ExpiryMaintenance::new(
            INTERVAL,
            ROWS_PER_RUN,
            // Readiness waits for one cleanup to commit under this login.
            PeriodicStartup::after_first_success(PATIENCE / 2)?,
            PeriodicShutdown::StopAtDrain,
        )?,
    )?;
    // Finalizers run in reverse registration order, so this probe runs before
    // the pool finalizer reserved above.
    let observed = Arc::new(Mutex::new(None));
    let probe = (
        limiter.maintenance().clone(),
        pool.clone(),
        observed.clone(),
    );
    supervisor.on_cleanup("probe.before.pool.close", move || async move {
        let (reader, pool, observed): (PeriodicReader, PgPool, _) = probe;
        *observed.lock().expect("probe lock") =
            Some((reader.snapshot().completion, pool.is_closed()));
        Ok(())
    })?;

    let running = supervisor.start();
    let observation = observe(&running, &limiter, owner, &quotas, family, &login).await;
    // Always settle, even after a failed observation, and report both.
    let settled = running.shutdown_checked().await;
    let success = match (observation, settled) {
        (Ok(()), Ok(success)) => success,
        (observation, settled) => {
            let settled = settled
                .map(|_| ())
                .map_err(|failure| fail(&format!("checked shutdown failed: {failure:?}")));
            both(observation, settled)?;
            return Err(fail("an observation failed"));
        }
    };
    let summary = &success.report().periodic[0].summary;
    require!(
        summary.completion == PeriodicCompletion::Stopped
            && summary.acknowledged
            && summary.recoverable_failures == 0
            && summary.deadline_exceeded == 0
            && summary.succeeded >= 3,
        &format!("unexpected maintenance evidence: {summary:?}"),
    )?;
    require!(
        *observed.lock().expect("probe lock") == Some((PeriodicCompletion::Stopped, false)),
        "maintenance had not stopped, or the pool was already closed, when finalizers began",
    )?;
    require!(
        pool.is_closed(),
        "the pool finalizer did not close the pool",
    )
}

/// Readiness after a committed cleanup, removal of counters that expired before
/// and after it, and a live counter that later runs leave in place.
async fn observe<L: Store>(
    running: &RunningSupervisor,
    limiter: &MaintainedLimiter<L>,
    owner: &PgPool,
    quotas: &str,
    family: Family,
    login: &str,
) -> Result {
    timeout(PATIENCE, running.status().wait_ready())
        .await
        .map_err(|_| fail("a maintained limiter did not become ready"))?
        .map_err(|readiness| {
            fail(&format!(
                "startup failed: {readiness:?}; retained cleanup failure: {}",
                retained_cause(limiter.maintenance())
            ))
        })?;
    let before_start = format!("{login}-before-start");
    eventually(
        "maintenance did not remove the expired counter",
        async || Ok(seeded_rows(owner, quotas, family, &before_start).await? == 0),
    )
    .await?;

    // A counter that expires after the first run proves recurrence.
    let after_ready = format!("{login}-after-ready");
    seed_expired_counter(owner, quotas, family, &after_ready).await?;
    eventually(
        "a later run did not remove a later expired counter",
        async || Ok(seeded_rows(owner, quotas, family, &after_ready).await? == 0),
    )
    .await?;

    // The same limiter serves; its live counter survives later runs.
    let key = L::admit(limiter, &format!("{login}-live")).await?;
    let invoked = limiter.maintenance().snapshot().invocations;
    eventually("maintenance stopped running", async || {
        Ok(limiter.maintenance().snapshot().invocations >= invoked + 2)
    })
    .await?;
    let live_rows: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT count(*) FROM {}.{} WHERE subject_key = $1",
        quote(quotas),
        family.counters()
    )))
    .bind(key.as_bytes().as_slice())
    .fetch_one(owner)
    .await?;
    require!(live_rows == 1, "maintenance removed a live counter")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires a dedicated disposable PostgreSQL 18 cluster through BATTER_SQLX_ADMIN_URL"]
async fn maintenance_without_its_cleanup_selection_never_becomes_ready() -> Result {
    Fixture::run(async |fixture| {
        schema::install(fixture).await?;
        refused::<PostgresLimiter>(fixture, Family::FixedWindow).await?;
        refused::<PostgresGcraLimiter>(fixture, Family::Gcra).await
    })
    .await
}

/// A login holding only admission cannot clean, so first-success startup must
/// withhold readiness and retain the native permission failure.
async fn refused<L: Store>(fixture: &mut Fixture, family: Family) -> Result {
    let quotas = fixture.names.quotas.clone();
    let role = Composition::new(PublicPolicy::Deny)
        .with_quotas(&quotas, &[family.admission()])
        .compile()?;
    let label = match family {
        Family::FixedWindow => "fixed_unmaintainable",
        Family::Gcra => "gcra_unmaintainable",
    };
    let login = fixture.login(label, &role).await?;
    let mut supervisor = supervisor()?;
    let pool = batter::sqlx::pool_in(
        supervisor.reserve_cleanup("quota.pool")?,
        PgPoolOptions::new().max_connections(2),
        quota_options(fixture, &login, &quotas)?,
    );
    fixture.track(pool.clone());
    let limiter = register_in(
        &mut supervisor,
        "quota.maintenance",
        L::on(pool),
        ExpiryMaintenance::new(
            INTERVAL,
            ROWS_PER_RUN,
            PeriodicStartup::after_first_success(Duration::from_secs(2))?,
            PeriodicShutdown::StopAtDrain,
        )?,
    )?;
    let running = supervisor.start();
    require!(
        timeout(PATIENCE, running.status().wait_ready())
            .await
            .map_err(|_| fail("readiness neither published nor refused"))?
            .is_err(),
        "a login without the cleanup selection became ready",
    )?;
    let report = running
        .wait_report()
        .await
        .map_err(|error| fail(&format!("coordinator failed: {error}")))?;
    let summary = &report.periodic[0].summary;
    require!(
        summary.completion == PeriodicCompletion::InitializationExpired && !summary.acknowledged,
        &format!("unexpected startup evidence: {summary:?}"),
    )?;
    let cause = limiter
        .maintenance()
        .snapshot()
        .first_failure
        .ok_or_else(|| fail("no failed cleanup was retained"))?
        .error;
    let code = match cause.downcast_ref::<MaintenanceError>() {
        Some(MaintenanceError::Database(error)) => error
            .as_database_error()
            .and_then(|database| database.code())
            .map(|code| code.into_owned()),
        _ => None,
    };
    require!(
        code.as_deref() == Some(DENIED),
        &format!("the retained cleanup failure was not a permission refusal: {code:?}"),
    )
}
