use std::time::Duration;

use runlimit_core::{Check, DecisionView, Denial};

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
        let decision = limiter.check(&check).await.unwrap();
        if allowed {
            assert!(
                matches!(decision.view(), DecisionView::Allowed { allowance } if allowance.available() == 0),
                "at {elapsed} ms: {decision:?}"
            );
        } else {
            assert!(
                matches!(decision.view(), DecisionView::Denied { denial: Denial::QuotaExceeded(quota) } if quota.retry_after().duration() == Duration::from_millis(retry_ms)),
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
