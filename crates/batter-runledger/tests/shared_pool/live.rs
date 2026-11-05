use super::*;
use runledger_test_support::{setup_unmigrated_ephemeral_pool, teardown_ephemeral_pool};

#[tokio::test]
async fn application_and_runledger_share_physical_capacity_and_awaited_close() {
    let (admin, fixture) = setup_unmigrated_ephemeral_pool("shared_pool_owner", 1).await;
    let options = (*admin.connect_options()).clone();
    let login = options.get_username();
    let policy = PgSessionProfile::new(
        login,
        login,
        vec!["public".into()],
        Duration::ZERO,
        Duration::ZERO,
    )
    .unwrap();
    let mut stack = CleanupStack::new();
    let shared = pool_in(
        stack.reserve("pool").unwrap(),
        PgPoolOptions::new().max_connections(1),
        options,
        policy,
    )
    .unwrap();
    let database = RunledgerDatabase::from_profiled(shared.clone()).unwrap();
    let mut app = shared.pool().acquire().await.unwrap();
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *app)
        .await
        .unwrap();
    assert_eq!(database.pool().size(), 1);
    assert!(
        database.pool().try_acquire().is_none(),
        "application owns the only slot"
    );
    drop(app);
    let mut worker = database.pool().acquire().await.unwrap();
    let worker_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *worker)
        .await
        .unwrap();
    assert_eq!(pid, worker_pid, "same physical session after profile reset");
    let closing = stack.close(cleanup());
    tokio::pin!(closing);
    assert!(
        tokio::time::timeout(Duration::from_millis(20), &mut closing)
            .await
            .is_err(),
        "close must await the worker checkout"
    );
    assert!(shared.pool().is_closed());
    drop(worker);
    let report = tokio::time::timeout(BOUND, closing).await.unwrap();
    assert!(report.is_success());
    assert_eq!(report.records.len(), 1);
    assert_eq!(shared.pool().size(), 0);
    assert!(matches!(
        database.pool().acquire().await,
        Err(sqlx::Error::PoolClosed)
    ));
    teardown_ephemeral_pool(admin, fixture).await;
}
