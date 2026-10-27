use super::*;
use online_support::{index_catalog, wait_for};
use runledger_postgres::{OnlineIndexExecutionError, OnlineIndexPhase, SchemaCompatibilityError};
use support::profiled_database;

fn execution(error: &SchemaCompatibilityError) -> &OnlineIndexExecutionError {
    let error = if let SchemaCompatibilityError::MigrationCleanup(pair) = error {
        pair.operation_error()
    } else {
        error
    };
    let SchemaCompatibilityError::OnlineIndexExecution(error) = error else {
        panic!("execution context: {error}")
    };
    assert_eq!(error.phase(), OnlineIndexPhase::Build);
    error
}

async fn run(database: &RunledgerDatabase, prepare: bool) -> Result<(), SchemaCompatibilityError> {
    if prepare {
        prepare_online_indexes_for_external_migrator(database)
            .await
            .map(|_| ())
    } else {
        migrate_after_idempotency_cutover(database).await
    }
}

#[tokio::test]
async fn busy_writer_refuses_metadata_drop_and_later_recovery_succeeds() {
    for prepare in [false, true] {
        let harness = Harness::fresh("online_repair_busy").await;
        let index = invalid_index(&harness.pool, prepare).await;
        let before = index_catalog(&harness.pool).await;
        let history = sqlx_history(&harness.pool).await;
        let writer = hold_writer_lock(&harness.pool).await;
        let database = profiled_database(&harness.pool).await;
        // Every NOWAIT attempt is refused while this writer outlasts the
        // documented five-second retry budget.
        let started = std::time::Instant::now();
        let error = tokio::time::timeout(Duration::from_secs(15), run(&database, prepare))
            .await
            .expect("bounded table lock refusal")
            .expect_err("writer keeps invalid object intact");
        assert!(
            started.elapsed() >= Duration::from_secs(4),
            "refused attempts are retried for the budget, not abandoned at once"
        );
        let SchemaCompatibilityError::Query(cause) = execution(&error).cause() else {
            panic!("native lock refusal")
        };
        assert_eq!(
            cause.as_database_error().and_then(|e| e.code()).as_deref(),
            Some("55P03")
        );
        assert_eq!(index_catalog(&harness.pool).await, before);
        assert_eq!(sqlx_history(&harness.pool).await, history);
        writer.rollback().await.expect("release writer");
        run(&database, prepare)
            .await
            .expect("repair after writer settles");
        assert!(index_catalog(&harness.pool).await[index].1);
        database.pool().close().await;
        harness.teardown().await;
    }
}

#[tokio::test]
async fn refused_metadata_attempt_is_retried_once_a_busy_writer_releases() {
    for prepare in [false, true] {
        let harness = Harness::fresh("online_repair_retry").await;
        let index = invalid_index(&harness.pool, prepare).await;
        let history = sqlx_history(&harness.pool).await;
        let writer = hold_writer_lock(&harness.pool).await;
        let mut relay = Relay::start(
            &harness.pool.connect_options(),
            Fault::ObserveRepairRollback,
        )
        .await;
        let database = relayed_database(&harness.pool, &relay);
        let task_database = database.clone();
        let task = tokio::spawn(async move { run(&task_database, prepare).await });
        // The first metadata attempt was refused and rolled back while the
        // writer held its lock; the same call must still repair the index.
        relay.reached().await;
        writer.rollback().await.expect("release writer");
        tokio::time::timeout(Duration::from_secs(10), task)
            .await
            .expect("retry within the budget")
            .expect("repair task")
            .expect("repair in the same call after the writer settles");
        assert!(index_catalog(&harness.pool).await[index].1);
        assert_eq!(sqlx_history(&harness.pool).await, history);
        database.pool().close().await;
        relay.finish().await;
        harness.teardown().await;
    }
}

#[tokio::test]
async fn lost_repair_commit_acknowledgement_retains_uncertainty_and_does_not_build() {
    for prepare in [false, true] {
        let harness = Harness::fresh("online_repair_commit").await;
        let index = invalid_index(&harness.pool, prepare).await;
        let history = sqlx_history(&harness.pool).await;
        let mut relay = Relay::start(
            &harness.pool.connect_options(),
            Fault::LoseRepairCommitResponse,
        )
        .await;
        let database = relayed_database(&harness.pool, &relay);
        let error = tokio::time::timeout(Duration::from_secs(10), run(&database, prepare))
            .await
            .expect("disconnection settles")
            .expect_err("unconfirmed commit");
        let failure = execution(&error);
        assert_eq!(failure.recorded_version().is_none(), prepare);
        let SchemaCompatibilityError::OnlineIndexRepairCommitUnconfirmed(commit) = failure.cause()
        else {
            panic!("commit uncertainty retained: {error}")
        };
        assert_eq!(commit.operation(), "remove interrupted online index");
        assert!(matches!(commit.sqlx_error(), sqlx::Error::Io(_)));
        assert!(failure.restoration_error().is_some());
        assert!(
            !index_catalog(&harness.pool).await.contains_key(index),
            "DROP committed but no build was dispatched"
        );
        assert_eq!(sqlx_history(&harness.pool).await, history);
        assert!(!format!("{error:?} {error}").contains("unexpected end"));
        database.pool().close().await;
        relay.finish().await;
        harness.teardown().await;
    }
}

#[tokio::test]
async fn failed_rollback_retains_original_lock_refusal_and_native_rollback_failure() {
    let harness = Harness::fresh("online_repair_rollback").await;
    invalid_index(&harness.pool, false).await;
    let before = index_catalog(&harness.pool).await;
    let writer = hold_writer_lock(&harness.pool).await;
    let mut relay = Relay::start(
        &harness.pool.connect_options(),
        Fault::DisconnectOnRepairRollback,
    )
    .await;
    let database = relayed_database(&harness.pool, &relay);
    let error = tokio::time::timeout(Duration::from_secs(10), run(&database, false))
        .await
        .expect("disconnection settles")
        .expect_err("rollback response lost");
    let SchemaCompatibilityError::OnlineIndexRollback(pair) = execution(&error).cause() else {
        panic!("independent causes retained: {error}")
    };
    let SchemaCompatibilityError::Query(cause) = pair.operation_error() else {
        panic!("native lock refusal")
    };
    assert_eq!(
        cause.as_database_error().and_then(|e| e.code()).as_deref(),
        Some("55P03")
    );
    assert!(matches!(pair.rollback_error(), sqlx::Error::Io(_)));
    assert!(!format!("{pair:?} {pair}").contains("could not obtain lock"));
    database.pool().close().await;
    relay.finish().await;
    writer.rollback().await.expect("release writer");
    assert_eq!(index_catalog(&harness.pool).await, before);
    harness.teardown().await;
}

#[tokio::test]
async fn cancelled_metadata_repair_leaves_original_name_and_no_backend_lock() {
    let harness = Harness::fresh("online_repair_cancel").await;
    invalid_index(&harness.pool, false).await;
    let before = index_catalog(&harness.pool).await;
    let mut relay = Relay::start(&harness.pool.connect_options(), Fault::HoldRepairTableLock).await;
    let database = relayed_database(&harness.pool, &relay);
    let task_database = database.clone();
    let task = tokio::spawn(async move { run(&task_database, false).await });
    relay.reached().await;
    let pid: i32 = sqlx::query_scalar("SELECT pid FROM pg_locks WHERE locktype='relation' AND relation='idx_job_queue_created_id'::regclass AND mode='ShareUpdateExclusiveLock' AND granted AND database=(SELECT oid FROM pg_database WHERE datname=current_database())").fetch_one(&harness.pool).await.expect("repair holds actual index lock");
    task.abort();
    assert!(task.await.expect_err("cancel task").is_cancelled());
    wait_for("remote transaction ends", || async {
        !sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE pid=$1)")
            .bind(pid)
            .fetch_one(&harness.pool)
            .await
            .expect("observe backend exit")
    })
    .await;
    assert_eq!(index_catalog(&harness.pool).await, before);
    database.pool().close().await;
    relay.finish().await;
    harness.teardown().await;
}

#[tokio::test]
async fn completed_manual_reindex_is_adopted_without_another_drop() {
    for prepare in [false, true] {
        let harness = Harness::fresh("online_repair_completed").await;
        let index = invalid_index(&harness.pool, prepare).await;
        let history = sqlx_history(&harness.pool).await;
        let mut relay = Relay::start(
            &harness.pool.connect_options(),
            Fault::HoldIndexInspection(index),
        )
        .await;
        let database = relayed_database(&harness.pool, &relay);
        let task_database = database.clone();
        let task = tokio::spawn(async move { run(&task_database, prepare).await });
        relay.reached().await;
        sqlx::raw_sql(AssertSqlSafe(format!("REINDEX INDEX CONCURRENTLY {index}")))
            .execute(&harness.pool)
            .await
            .expect("manual repair wins the race");
        let oid: i64 = sqlx::query_scalar("SELECT to_regclass($1)::oid::bigint")
            .bind(index)
            .fetch_one(&harness.pool)
            .await
            .expect("completed repair oid");
        relay.release_response().await;
        tokio::time::timeout(Duration::from_secs(10), task)
            .await
            .expect("adopt without competing DROP")
            .expect("task")
            .expect("adopt valid index");
        let after: i64 = sqlx::query_scalar("SELECT to_regclass($1)::oid::bigint")
            .bind(index)
            .fetch_one(&harness.pool)
            .await
            .expect("retained oid");
        assert_eq!(after, oid);
        assert_eq!(sqlx_history(&harness.pool).await, history);
        database.pool().close().await;
        relay.finish().await;
        harness.teardown().await;
    }
}
