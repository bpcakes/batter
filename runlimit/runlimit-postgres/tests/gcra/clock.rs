use std::time::Duration;

use runlimit_core::{
    Allowance, BatchDecisionView, Check, Decision, DecisionView, Denial, GcraPolicy, QuotaMode,
};
use runlimit_memory::{Clock, GcraStore, MemoryStoreConfig};
use sqlx::types::chrono::{DateTime, Utc};

use super::{Database, policy, subject};

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn database_time_replenishes_continuously_without_window_boundary_burst() {
    let db = Database::new().await;
    // Advance the persisted database clock explicitly. Real sleeps cannot bound
    // the elapsed time between requests on a delayed CI runner.
    let now = db.freeze_time().await;
    let policy = policy(2, 400, 2);
    let check = Check::new(subject(1, 0, &policy).bind(&policy));
    let limiter = db.limiter();
    assert!(
        limiter
            .check(&check.with_cost(2).unwrap())
            .await
            .unwrap()
            .permits_request()
    );
    // Two units per 400 ms replenish one unit at each 200 ms boundary.
    // Denials must not consume quota, and crossing 400 ms must not reset a burst.
    for (elapsed, allowed, retry_ms) in [
        (0, false, 200),
        (199, false, 1),
        (200, true, 0),
        (200, false, 200),
        (399, false, 1),
        (400, true, 0),
        (400, false, 200),
    ] {
        sqlx::query("UPDATE runlimit_gcra_shards SET observed_at_ms = $1 WHERE capacity_shard = 0")
            .bind(i64::try_from(now + elapsed).unwrap())
            .execute(&db.pool)
            .await
            .unwrap();
        let (decision, elapsed_bound) = check_with_elapsed_bound(&db, &check).await;
        if allowed {
            assert!(
                matches!(decision.view(), DecisionView::Allowed { allowance } if allowance.available() == 0),
                "at {elapsed} ms: {decision:?}"
            );
        } else {
            assert!(
                matches!(decision.view(), DecisionView::Denied { denial: Denial::QuotaExceeded(quota) } if delay_within_elapsed_bound(quota.retry_after().duration(), Duration::from_millis(retry_ms), elapsed_bound)),
                "at {elapsed} ms: {decision:?}"
            );
        }
    }
    db.teardown().await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn admission_persists_a_fresh_database_clock_sample() {
    let db = Database::new().await;
    let policy = policy(2, 400, 2);
    let check = Check::new(subject(1, 0, &policy).bind(&policy));
    let before = database_wall_time(&db).await;
    assert!(db.limiter().check(&check).await.unwrap().permits_request());
    let observed = db.watermark().await;
    let after = database_wall_time(&db).await;
    assert!(
        (before..=after).contains(&observed),
        "database sample {observed} is outside {before}..={after}"
    );
    db.teardown().await;
}

async fn database_wall_time(db: &Database) -> i64 {
    sqlx::query_scalar(
        "SELECT floor(extract(epoch FROM pg_catalog.clock_timestamp()) * 1000)::BIGINT",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap()
}

// Bound the subtraction using the same database clock around the entire call.
// The future shard watermark freezes evaluation, not clock_timestamp().
pub(super) async fn check_with_elapsed_bound(
    db: &Database,
    check: &Check<'_, GcraPolicy>,
) -> (Decision, Duration) {
    let before = database_timestamp(db).await;
    let decision = db.limiter().check(check).await.unwrap();
    let after = database_timestamp(db).await;
    (decision, (after - before).to_std().unwrap())
}

pub(super) fn delay_within_elapsed_bound(
    delay: Duration,
    expected: Duration,
    bound: Duration,
) -> bool {
    (expected.saturating_sub(bound)..=expected).contains(&delay)
}

pub(super) async fn database_timestamp(db: &Database) -> DateTime<Utc> {
    sqlx::query_scalar("SELECT pg_catalog.clock_timestamp()")
        .fetch_one(&db.pool)
        .await
        .unwrap()
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn replenishment_subtracts_time_through_the_last_watermark_update() {
    let db = Database::new().await;
    let now = db.freeze_time().await;
    let policy = policy(2, 2000, 2);
    let checks = [
        Check::new(subject(1, 0, &policy).bind(&policy)),
        Check::new(subject(2, 1, &policy).bind(&policy))
            .with_cost(2)
            .unwrap(),
    ];
    // Both rows must be updated before the response sample is evaluated.
    sqlx::raw_sql("CREATE FUNCTION slow_watermark() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN PERFORM pg_catalog.pg_sleep(0.05); RETURN NEW; END $$;
        CREATE TRIGGER slow_watermark BEFORE UPDATE OF observed_at_ms ON runlimit_gcra_shards FOR EACH ROW EXECUTE FUNCTION slow_watermark()")
        .execute(&db.pool).await.unwrap();
    let before = database_timestamp(&db).await;
    let decision = db.limiter().check_all(&checks).await.unwrap();
    let bound = (database_timestamp(&db).await - before).to_std().unwrap();
    let BatchDecisionView::Allowed { allowances } = decision.view() else {
        panic!("expected allowed batch")
    };
    for (index, allowance) in allowances.iter().enumerate() {
        let expected_ms = if index == 0 { 1000 } else { 2000 };
        let expected = Duration::from_millis(expected_ms);
        let delay = allowance.replenishes_after().duration();
        assert!(delay_within_elapsed_bound(delay, expected, bound));
        assert!(
            delay <= expected.checked_sub(Duration::from_millis(100)).unwrap(),
            "both delayed updates must be subtracted: {delay:?}"
        );
        assert_eq!(allowance.available(), u64::from(index == 0));
        let (tat, expiry, observed): (String, i64, i64) = sqlx::query_as("SELECT counter.tat_scaled::TEXT, counter.expires_at_ms, shard.observed_at_ms FROM runlimit_gcra AS counter JOIN runlimit_gcra_shards AS shard USING (capacity_shard) WHERE counter.subject_key = $1")
            .bind(checks[index].subject().as_bytes().as_slice()).fetch_one(&db.pool).await.unwrap();
        assert_eq!(
            tat.parse::<u128>().unwrap(),
            u128::from(now + expected_ms) * 2
        );
        assert_eq!(u64::try_from(expiry).unwrap(), now + expected_ms);
        assert_eq!(u64::try_from(observed).unwrap(), now);
    }
    db.teardown().await;
}

pub(super) fn assert_allowance_matches_evaluation(
    actual: Allowance,
    expected: Allowance,
    bound: Duration,
) {
    assert_eq!(actual.capacity(), expected.capacity());
    assert_eq!(actual.available(), expected.available());
    assert!(delay_within_elapsed_bound(
        actual.replenishes_after().duration(),
        expected.replenishes_after().duration(),
        bound
    ));
}

pub(super) fn assert_decision_matches_evaluation(
    actual: &Decision,
    expected: &Decision,
    bound: Duration,
) {
    match (actual.view(), expected.view()) {
        (
            DecisionView::Allowed { allowance: actual },
            DecisionView::Allowed {
                allowance: expected,
            },
        ) => assert_allowance_matches_evaluation(actual, expected, bound),
        (
            DecisionView::Denied {
                denial: Denial::QuotaExceeded(actual),
            },
            DecisionView::Denied {
                denial: Denial::QuotaExceeded(expected),
            },
        )
        | (
            DecisionView::ShadowDenied { denial: actual },
            DecisionView::ShadowDenied { denial: expected },
        ) => {
            assert_eq!(actual.capacity(), expected.capacity());
            assert!(delay_within_elapsed_bound(
                actual.retry_after().duration(),
                expected.retry_after().duration(),
                bound
            ));
        }
        _ => panic!("decision mismatch: {actual:?} vs {expected:?}"),
    }
}

#[derive(Clone, Copy)]
struct FrozenClock(u64);
impl Clock for FrozenClock {
    fn now(&self) -> Duration {
        Duration::from_millis(self.0)
    }
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn exact_memory_parity_covers_fractional_refill_weighted_costs_and_shadow_mode() {
    for mode in [QuotaMode::Enforce, QuotaMode::Shadow] {
        let db = Database::new().await;
        let now = db.freeze_time().await;
        let memory = GcraStore::builder(MemoryStoreConfig::new(8).unwrap())
            .with_clock(FrozenClock(now))
            .build();
        let policy = policy(3, 10, 7).with_quota_mode(mode);
        let limiter = db.limiter();
        for cost in [3, 2, 3, 1, 2, 1] {
            let check = Check::new(subject(1, 0, &policy).bind(&policy))
                .with_cost(cost)
                .unwrap();
            let (actual, elapsed_bound) = check_with_elapsed_bound(&db, &check).await;
            assert_decision_matches_evaluation(
                &actual,
                &memory.check(&check).unwrap(),
                elapsed_bound,
            );
        }
        let checks = [
            Check::new(subject(2, 0, &policy).bind(&policy))
                .with_cost(4)
                .unwrap(),
            Check::new(subject(3, 1, &policy).bind(&policy))
                .with_cost(5)
                .unwrap(),
        ];
        let before = database_timestamp(&db).await;
        let actual = limiter.check_all(&checks).await.unwrap();
        let elapsed_bound = (database_timestamp(&db).await - before).to_std().unwrap();
        let expected = memory.check_all(&checks).unwrap();
        let (
            BatchDecisionView::Allowed { allowances: actual },
            BatchDecisionView::Allowed {
                allowances: expected,
            },
        ) = (actual.view(), expected.view())
        else {
            panic!("expected allowed batches")
        };
        assert_eq!(actual.len(), expected.len());
        for (actual, expected) in actual.iter().zip(expected) {
            assert_allowance_matches_evaluation(*actual, *expected, elapsed_bound);
        }
        db.teardown().await;
    }
}
