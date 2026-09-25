use super::*;
use std::{
    future::{Future, poll_fn},
    task::Poll,
};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires RUNLIMIT_POSTGRES_TEST_DATABASE_URL"]
async fn pool_wait_does_not_spend_admission_or_cleanup_operation_budget() {
    let fixture = IsolatedSchema::for_cleanup_test(1).await;
    let setup_pool = fixture.admin_pool.clone();
    let pool = fixture.primary_pool.clone();
    let schema = fixture.schema.clone();
    let policy = unique_policy("pool-budget-survivor", 1, Duration::from_mins(1));
    let subject = key(155);
    create_pool_budget_delay_triggers(&setup_pool, &schema).await;

    let operation_budget = Duration::from_secs(2);
    let pool_wait = operation_budget + Duration::from_millis(250);
    let config = PostgresConfig::new()
        .with_pool_acquire_timeout(Duration::from_secs(10))
        .expect("ten-second pool budget is valid")
        .with_operation_timeout(operation_budget)
        .expect("two-second operation budget is valid");
    let limiter = PostgresLimiter::new(pool.clone()).with_config(config);

    let held_connection = pool
        .acquire()
        .await
        .expect("hold the only pool connection before admission");
    let check = Check::new(subject.bind(&policy));
    let mut admission = Box::pin(limiter.check(&check));
    assert!(
        poll_fn(|cx| Poll::Ready(admission.as_mut().poll(cx)))
            .await
            .is_pending()
    );
    // Exceed the work budget only after polling into the pool wait. An early
    // deadline must expire; a fresh one leaves headroom for delayed SQL and commit.
    sleep(pool_wait).await;
    drop(held_connection);

    let decision = tokio::time::timeout(Duration::from_secs(10), admission)
        .await
        .expect("admission completes within the watchdog")
        .expect("admission receives a fresh operation budget after pool wait");
    assert!(decision.permits_request());
    assert_eq!(available(&decision), 0);

    sqlx::query(
        r"
UPDATE runlimit_fixed_windows
SET
    window_started_at = pg_catalog.clock_timestamp() - INTERVAL '2 seconds',
    window_expires_at = pg_catalog.clock_timestamp() - INTERVAL '1 second'
WHERE
    config_fingerprint = $1
    AND subject_key = $2
",
    )
    .bind(policy.fingerprint().as_bytes().as_slice())
    .bind(subject.as_bytes().as_slice())
    .execute(&pool)
    .await
    .expect("expire the admitted counter before cleanup");

    let held_connection = pool
        .acquire()
        .await
        .expect("hold the only pool connection before cleanup");
    let mut cleanup = Box::pin(limiter.cleanup_expired(1));
    assert!(
        poll_fn(|cx| Poll::Ready(cleanup.as_mut().poll(cx)))
            .await
            .is_pending()
    );
    sleep(pool_wait).await;
    drop(held_connection);

    let removed = tokio::time::timeout(Duration::from_secs(10), cleanup)
        .await
        .expect("cleanup completes within the watchdog")
        .expect("cleanup receives a fresh operation budget after pool wait");
    assert_eq!(removed, 1);
    assert!(!counter_exists(&pool, &policy, subject).await);

    drop(limiter);
    fixture.teardown().await;
}
