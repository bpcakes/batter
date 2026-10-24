//! Live evidence that the protected migration path builds the declared
//! queue indexes online while recording the canonical migrations, repairs a
//! lost or `INVALID` index, refuses a foreign one, and that the external
//! preparation path prebuilds only what canonical SQL adopts.

use std::time::Duration;

use runledger_postgres::{
    MIGRATOR, OnlineIndexProblem, SchemaCompatibilityError,
    prepare_online_indexes_for_external_migrator,
};
mod online_support;
mod support;
use online_support::{
    DECLARED_INDEXES, Harness, LAST_VERSION_BEFORE_CUTOVER, LAST_VERSION_BEFORE_SNAPSHOT_COLUMNS,
    LAST_VERSION_BEFORE_SUMMARY_PAGINATION, SUMMARY_PAGINATION_VERSION,
    assert_declared_indexes_valid, build_waits_for_writers, hold_writer_lock, index_catalog,
    migrate_with_profile_timeouts, migrate_with_short_timeouts, raw_apply_through, sqlx_history,
    wait_for, writer_can_lock,
};
use sqlx::Connection;
use sqlx::migrate::Migrate;
use support::{migrate_profiled, profiled_database, verify_profiled};

#[tokio::test]
async fn protected_runner_records_canonical_history_and_builds_identical_indexes() {
    let raw = Harness::fresh("online_idx_raw").await;
    let protected = Harness::fresh("online_idx_protected").await;

    MIGRATOR
        .run(&raw.pool)
        .await
        .expect("raw canonical migrator");
    migrate_profiled(&protected.pool)
        .await
        .expect("protected runner on a fresh database");

    assert_eq!(
        sqlx_history(&raw.pool).await,
        sqlx_history(&protected.pool).await,
        "the protected runner must record every canonical version and checksum"
    );
    let raw_catalog = index_catalog(&raw.pool).await;
    let protected_catalog = index_catalog(&protected.pool).await;
    assert_eq!(
        raw_catalog, protected_catalog,
        "online builds must leave the same index definitions as canonical builds"
    );
    assert_declared_indexes_valid(&protected.pool, "fresh protected run").await;
    verify_profiled(&protected.pool)
        .await
        .expect("read-only verification accepts the online-built schema");
    migrate_profiled(&protected.pool)
        .await
        .expect("repeated protected startup is idempotent");

    raw.teardown().await;
    protected.teardown().await;
}

#[tokio::test]
async fn external_preparation_prebuilds_only_adoptable_indexes_and_runner_adopts_them() {
    let harness = Harness::fresh("online_idx_external").await;

    let report =
        prepare_online_indexes_for_external_migrator(&profiled_database(&harness.pool).await)
            .await
            .expect("preparation on an empty database");
    assert!(report.built().is_empty());
    assert_eq!(report.deferred(), DECLARED_INDEXES, "no table exists yet");
    assert!(report.external_build_blocks().is_empty());

    // The tables exist but the snapshot columns the cutover indexes reference
    // do not: preparation defers instead of failing on the missing column.
    raw_apply_through(&harness.pool, LAST_VERSION_BEFORE_SNAPSHOT_COLUMNS).await;
    let database = profiled_database(&harness.pool).await;
    let report = prepare_online_indexes_for_external_migrator(&database)
        .await
        .expect("preparation before the snapshot-column migration");
    assert!(report.built().is_empty());
    assert_eq!(
        report.deferred(),
        &DECLARED_INDEXES[..2],
        "the adoptable pair waits for the earlier pending migration"
    );
    assert_eq!(
        report.external_build_blocks(),
        &DECLARED_INDEXES[2..],
        "indexes on an existing table still block under external tooling"
    );
    database.pool().close().await;

    raw_apply_through(&harness.pool, LAST_VERSION_BEFORE_CUTOVER).await;
    let database = profiled_database(&harness.pool).await;
    let report = prepare_online_indexes_for_external_migrator(&database)
        .await
        .expect("preparation before the cutover migration");
    assert_eq!(report.built(), &DECLARED_INDEXES[..2]);
    assert!(report.already_valid().is_empty());
    assert!(report.deferred().is_empty());
    assert_eq!(
        report.external_build_blocks(),
        &DECLARED_INDEXES[2..],
        "indexes without IF NOT EXISTS stay blocking under external tooling"
    );
    let report = prepare_online_indexes_for_external_migrator(&database)
        .await
        .expect("repeated preparation");
    assert!(report.built().is_empty());
    assert_eq!(report.already_valid(), &DECLARED_INDEXES[..2]);
    database.pool().close().await;

    // The canonical cutover SQL adopts the prebuilt pair; a raw migrator keeps
    // its IF NOT EXISTS statements as no-ops.
    let before = index_catalog(&harness.pool).await;
    raw_apply_through(&harness.pool, 202605220001).await;
    let after = index_catalog(&harness.pool).await;
    for name in &DECLARED_INDEXES[..2] {
        assert_eq!(
            before.get(*name),
            after.get(*name),
            "{name} adopted unchanged"
        );
    }

    migrate_profiled(&harness.pool)
        .await
        .expect("protected runner completes the remaining migrations");
    assert_declared_indexes_valid(&harness.pool, "after protected completion").await;
    let database = profiled_database(&harness.pool).await;
    let report = prepare_online_indexes_for_external_migrator(&database)
        .await
        .expect("preparation after everything is applied");
    assert!(report.built().is_empty() && report.already_valid().is_empty());
    assert!(report.deferred().is_empty() && report.external_build_blocks().is_empty());
    database.pool().close().await;

    harness.teardown().await;
}

#[tokio::test]
async fn protected_runner_repairs_missing_and_invalid_indexes_that_verification_reports() {
    let harness = Harness::fresh("online_idx_repair").await;
    migrate_profiled(&harness.pool)
        .await
        .expect("initial migration");

    sqlx::raw_sql("DROP INDEX idx_job_queue_created_id; DROP INDEX idx_job_queue_scope_created_id")
        .execute(&harness.pool)
        .await
        .expect("simulate an interrupted build window");
    // Leave an INVALID index: a concurrent build cancelled while it waits for
    // an open writer transaction keeps its catalog entry.
    let holder = hold_writer_lock(&harness.pool).await;
    let mut builder = harness.pool.acquire().await.expect("builder connection");
    sqlx::raw_sql("SET statement_timeout = '300ms'")
        .execute(&mut *builder)
        .await
        .expect("bound the build");
    let cancelled = sqlx::raw_sql(
        "CREATE INDEX CONCURRENTLY idx_job_queue_scope_created_id ON job_queue (organization_id, created_at DESC, id DESC)",
    )
    .execute(&mut *builder)
    .await
    .expect_err("the build must be cancelled by statement_timeout");
    assert_eq!(
        cancelled
            .as_database_error()
            .and_then(|error| error.code())
            .as_deref(),
        Some("57014"),
        "expected query_canceled, got {cancelled}"
    );
    drop(builder);
    holder.rollback().await.expect("release writer lock");
    let catalog = index_catalog(&harness.pool).await;
    assert!(
        matches!(
            catalog.get("idx_job_queue_scope_created_id"),
            Some((_, false))
        ),
        "cancelled concurrent build must leave an INVALID index"
    );
    assert!(!catalog.contains_key("idx_job_queue_created_id"));

    let error = verify_profiled(&harness.pool)
        .await
        .expect_err("read-only verification must report the damaged index");
    assert!(
        matches!(
            error,
            SchemaCompatibilityError::OnlineIndex {
                index: "idx_job_queue_scope_created_id",
                problem: OnlineIndexProblem::Invalid
            }
        ),
        "unexpected error: {error}"
    );

    migrate_profiled(&harness.pool)
        .await
        .expect("protected startup rebuilds the damaged indexes");
    let repaired = index_catalog(&harness.pool).await;
    assert!(matches!(
        repaired.get("idx_job_queue_scope_created_id"),
        Some((_, true))
    ));
    assert!(matches!(
        repaired.get("idx_job_queue_created_id"),
        Some((_, true))
    ));
    verify_profiled(&harness.pool)
        .await
        .expect("verification accepts the repair");

    sqlx::raw_sql("DROP INDEX idx_job_queue_created_id")
        .execute(&harness.pool)
        .await
        .expect("drop again");
    let error = verify_profiled(&harness.pool)
        .await
        .expect_err("missing index is reported");
    assert!(
        matches!(
            error,
            SchemaCompatibilityError::OnlineIndex {
                index: "idx_job_queue_created_id",
                problem: OnlineIndexProblem::Missing
            }
        ),
        "unexpected error: {error}"
    );

    harness.teardown().await;
}

#[tokio::test]
async fn protected_runner_refuses_a_foreign_index_under_a_declared_name() {
    let harness = Harness::fresh("online_idx_foreign").await;
    raw_apply_through(&harness.pool, LAST_VERSION_BEFORE_SUMMARY_PAGINATION).await;
    sqlx::raw_sql("CREATE INDEX idx_job_queue_created_id ON job_queue (id)")
        .execute(&harness.pool)
        .await
        .expect("foreign index");

    let error = migrate_profiled(&harness.pool)
        .await
        .expect_err("a differently defined index under a declared name is refused");
    assert!(
        matches!(
            error,
            SchemaCompatibilityError::OnlineIndex {
                index: "idx_job_queue_created_id",
                problem: OnlineIndexProblem::DefinitionMismatch
            }
        ),
        "unexpected error: {error}"
    );
    assert!(
        !sqlx_history(&harness.pool)
            .await
            .iter()
            .any(|(version, _, _)| *version == SUMMARY_PAGINATION_VERSION),
        "the refused migration must not be recorded"
    );

    sqlx::raw_sql("DROP INDEX idx_job_queue_created_id")
        .execute(&harness.pool)
        .await
        .expect("remove foreign index");
    migrate_profiled(&harness.pool)
        .await
        .expect("runner proceeds once the name is free");
    assert_declared_indexes_valid(&harness.pool, "after foreign index removal").await;

    harness.teardown().await;
}

#[tokio::test]
async fn online_build_waits_without_stalling_writers_unlike_the_canonical_build() {
    // Protected path: the concurrent build waits for the open writer, and a
    // second writer still acquires its lock.
    let harness = Harness::fresh("online_idx_writers").await;
    raw_apply_through(&harness.pool, LAST_VERSION_BEFORE_SUMMARY_PAGINATION).await;
    let holder = hold_writer_lock(&harness.pool).await;
    let runner_pool = harness.pool.clone();
    let runner = tokio::spawn(async move { migrate_profiled(&runner_pool).await });
    wait_for("the concurrent build to wait on the writer", || {
        build_waits_for_writers(&harness.pool)
    })
    .await;
    writer_can_lock(&harness.pool)
        .await
        .expect("a writer must not queue behind the concurrent build");
    holder.commit().await.expect("release writer");
    runner
        .await
        .expect("runner task")
        .expect("protected runner completes after the writer commits");
    assert_declared_indexes_valid(&harness.pool, "after online build under load").await;
    harness.teardown().await;

    // Canonical path: the ordinary build queues a SHARE lock behind the open
    // writer, and a second writer times out behind that queued request.
    let harness = Harness::fresh("online_idx_canonical_writers").await;
    raw_apply_through(&harness.pool, LAST_VERSION_BEFORE_SUMMARY_PAGINATION).await;
    let holder = hold_writer_lock(&harness.pool).await;
    let raw_pool = harness.pool.clone();
    let raw_runner =
        tokio::spawn(async move { raw_apply_through(&raw_pool, SUMMARY_PAGINATION_VERSION).await });
    wait_for("the canonical build to queue its SHARE lock", || async {
        sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (
                SELECT 1 FROM pg_catalog.pg_locks
                WHERE relation = 'job_queue'::regclass
                  AND mode = 'ShareLock'
                  AND NOT granted
             )",
        )
        .fetch_one(&harness.pool)
        .await
        .expect("lock view")
    })
    .await;
    let stalled = writer_can_lock(&harness.pool)
        .await
        .expect_err("a writer queues behind the canonical SHARE lock request");
    assert_eq!(
        stalled
            .as_database_error()
            .and_then(|error| error.code())
            .as_deref(),
        Some("55P03"),
        "expected lock_not_available, got {stalled}"
    );
    holder.commit().await.expect("release writer");
    raw_runner.await.expect("raw runner task");
    harness.teardown().await;
}

#[tokio::test]
async fn concurrent_protected_startups_both_succeed_with_pending_online_indexes() {
    let harness = Harness::fresh("online_idx_concurrent").await;
    raw_apply_through(&harness.pool, LAST_VERSION_BEFORE_SUMMARY_PAGINATION).await;

    // Hold a writer lock so the first starter's build waits; the second
    // starter then provably waits for the lock while that build is running.
    let holder = hold_writer_lock(&harness.pool).await;
    let first_pool = harness.pool.clone();
    let first = tokio::spawn(async move { migrate_profiled(&first_pool).await });
    wait_for(
        "the first starter's build to wait on the writer",
        || async {
            sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS (
                SELECT 1 FROM pg_catalog.pg_stat_progress_create_index
                WHERE command = 'CREATE INDEX CONCURRENTLY'
                  AND relid = 'job_queue'::regclass
                  AND phase LIKE 'waiting for%'
             )",
            )
            .fetch_one(&harness.pool)
            .await
            .expect("progress view")
        },
    )
    .await;
    let second_pool = harness.pool.clone();
    let second = tokio::spawn(async move { migrate_profiled(&second_pool).await });
    // Match the starter's exact poll statement: this probe's own text never
    // equals it, so an idle pooled connection cannot satisfy the wait.
    wait_for("the second starter to poll for the lock", || async {
        sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (
                SELECT 1 FROM pg_catalog.pg_stat_activity
                WHERE query = 'SELECT pg_catalog.pg_try_advisory_lock($1)'
             )",
        )
        .fetch_one(&harness.pool)
        .await
        .expect("activity view")
    })
    .await;
    let blocking_waiters: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_catalog.pg_locks WHERE locktype = 'advisory' AND NOT granted",
    )
    .fetch_one(&harness.pool)
    .await
    .expect("lock view");
    assert_eq!(
        blocking_waiters, 0,
        "the second starter must poll, not queue a blocking advisory lock behind the build"
    );
    holder.commit().await.expect("release writer");
    let (first, second) = tokio::join!(first, second);
    first
        .expect("first starter task")
        .expect("first concurrent protected startup succeeds");
    second
        .expect("second starter task")
        .expect("second concurrent protected startup succeeds");

    assert_declared_indexes_valid(&harness.pool, "after concurrent startups").await;
    let recorded = sqlx_history(&harness.pool)
        .await
        .into_iter()
        .filter(|(version, _, success)| *version == SUMMARY_PAGINATION_VERSION && *success)
        .count();
    assert_eq!(
        recorded, 1,
        "the pending migration is recorded exactly once"
    );
    verify_profiled(&harness.pool)
        .await
        .expect("read-only verification after concurrent startups");
    harness.teardown().await;
}

#[tokio::test]
async fn runledger_lock_wait_excludes_sqlx_holders_without_queueing_a_blocking_lock() {
    let harness = Harness::fresh("online_idx_lock_key").await;
    raw_apply_through(&harness.pool, LAST_VERSION_BEFORE_CUTOVER).await;

    // Hold the migration lock exactly as SQLx's own migrator does.
    let mut holder = sqlx::PgConnection::connect(harness.database.url())
        .await
        .expect("holder connection");
    holder.lock().await.expect("SQLx migration lock");

    let database = profiled_database(&harness.pool).await;
    let mut preparation =
        tokio::spawn(async move { prepare_online_indexes_for_external_migrator(&database).await });
    let elapsed = tokio::time::timeout(Duration::from_millis(1500), &mut preparation).await;
    assert!(
        elapsed.is_err(),
        "preparation must wait while SQLx holds the lock: the derived key matches"
    );
    let blocking_waiters: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_catalog.pg_locks WHERE locktype = 'advisory' AND NOT granted",
    )
    .fetch_one(&harness.pool)
    .await
    .expect("lock view");
    assert_eq!(
        blocking_waiters, 0,
        "the waiting starter must poll instead of queueing a blocking advisory lock"
    );
    let waiting_with_snapshot: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_catalog.pg_stat_activity
         WHERE wait_event_type = 'Lock' AND backend_xmin IS NOT NULL",
    )
    .fetch_one(&harness.pool)
    .await
    .expect("activity view");
    assert_eq!(
        waiting_with_snapshot, 0,
        "no session waits on a lock while holding a snapshot"
    );

    holder.unlock().await.expect("release SQLx lock");
    let report = preparation
        .await
        .expect("preparation task")
        .expect("preparation completes once SQLx releases the lock");
    assert_eq!(report.built(), &DECLARED_INDEXES[..2]);
    holder.close().await.expect("close holder");
    harness.teardown().await;
}

#[tokio::test]
async fn online_build_outlasts_the_profile_timeouts_while_it_waits_for_writers() {
    let harness = Harness::fresh("online_idx_timeouts").await;
    raw_apply_through(&harness.pool, LAST_VERSION_BEFORE_SUMMARY_PAGINATION).await;
    let holder = hold_writer_lock(&harness.pool).await;
    let runner_pool = harness.pool.clone();
    let runner = tokio::spawn(async move { migrate_with_short_timeouts(&runner_pool).await });
    wait_for("the concurrent build to wait on the writer", || {
        build_waits_for_writers(&harness.pool)
    })
    .await;
    // Outlast both the 250 ms lock timeout and the 2 s statement timeout.
    tokio::time::sleep(Duration::from_millis(3000)).await;
    assert!(
        !runner.is_finished(),
        "the build must still be waiting, not cancelled"
    );
    holder.commit().await.expect("release writer");
    runner
        .await
        .expect("runner task")
        .expect("the online build is not bounded by the profile timeouts");
    assert_declared_indexes_valid(&harness.pool, "after a build that outlasted the timeouts").await;
    verify_profiled(&harness.pool)
        .await
        .expect("verification after the timed-out-profile build");
    harness.teardown().await;
}

#[tokio::test]
async fn verification_reports_a_build_in_progress_instead_of_an_interrupted_one() {
    let harness = Harness::fresh("online_idx_in_progress").await;
    raw_apply_through(&harness.pool, LAST_VERSION_BEFORE_SUMMARY_PAGINATION).await;
    let holder = hold_writer_lock(&harness.pool).await;
    let runner_pool = harness.pool.clone();
    let runner = tokio::spawn(async move { migrate_profiled(&runner_pool).await });
    wait_for("the concurrent build to wait on the writer", || {
        build_waits_for_writers(&harness.pool)
    })
    .await;
    let error = verify_profiled(&harness.pool)
        .await
        .expect_err("verification during another session's build is a transient refusal");
    assert!(
        matches!(
            error,
            SchemaCompatibilityError::OnlineIndex {
                index: "idx_job_queue_scope_created_id",
                problem: OnlineIndexProblem::BuildInProgress
            }
        ),
        "unexpected error: {error}"
    );
    holder.commit().await.expect("release writer");
    runner
        .await
        .expect("runner task")
        .expect("protected runner completes after the writer commits");
    verify_profiled(&harness.pool)
        .await
        .expect("verification passes once the build ended");
    harness.teardown().await;
}

#[tokio::test]
async fn protected_runner_refuses_a_same_named_index_on_another_table() {
    let harness = Harness::fresh("online_idx_foreign_table").await;
    raw_apply_through(&harness.pool, LAST_VERSION_BEFORE_SUMMARY_PAGINATION).await;
    sqlx::raw_sql("CREATE INDEX idx_job_queue_created_id ON job_attempts (id)")
        .execute(&harness.pool)
        .await
        .expect("same-named index on another table");

    let error = migrate_profiled(&harness.pool)
        .await
        .expect_err("a same-named index on another table is refused before recording");
    assert!(
        matches!(
            error,
            SchemaCompatibilityError::OnlineIndex {
                index: "idx_job_queue_created_id",
                problem: OnlineIndexProblem::DefinitionMismatch
            }
        ),
        "unexpected error: {error}"
    );
    assert!(
        !sqlx_history(&harness.pool)
            .await
            .iter()
            .any(|(version, _, _)| *version == SUMMARY_PAGINATION_VERSION),
        "the refused migration must not be recorded"
    );

    sqlx::raw_sql("DROP INDEX idx_job_queue_created_id")
        .execute(&harness.pool)
        .await
        .expect("remove the foreign index");
    migrate_profiled(&harness.pool)
        .await
        .expect("runner proceeds once the name is free");
    assert_declared_indexes_valid(&harness.pool, "after foreign-table index removal").await;
    harness.teardown().await;
}

#[tokio::test]
async fn lock_wait_honors_the_profile_lock_timeout() {
    let harness = Harness::fresh("online_idx_lock_timeout").await;
    raw_apply_through(&harness.pool, LAST_VERSION_BEFORE_SUMMARY_PAGINATION).await;
    let mut holder = sqlx::PgConnection::connect(harness.database.url())
        .await
        .expect("holder connection");
    holder.lock().await.expect("SQLx migration lock");

    let runner_pool = harness.pool.clone();
    let started = tokio::time::Instant::now();
    let error = tokio::time::timeout(
        Duration::from_secs(10),
        tokio::spawn(async move { migrate_with_short_timeouts(&runner_pool).await }),
    )
    .await
    .expect("the bounded wait must not hang")
    .expect("runner task")
    .expect_err("a 250 ms lock_timeout bounds the wait for a held migration lock");
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "expiry must follow the profile bound, not the test deadline"
    );
    assert!(
        matches!(error, SchemaCompatibilityError::Incompatible(_)),
        "unexpected error: {error}"
    );
    assert!(
        error.to_string().contains("lock_timeout"),
        "unexpected message: {error}"
    );

    holder.unlock().await.expect("release SQLx lock");
    holder.close().await.expect("close holder");
    migrate_profiled(&harness.pool)
        .await
        .expect("startup proceeds once the lock is free");
    harness.teardown().await;
}

#[tokio::test]
async fn verification_reports_a_missing_index_as_in_progress_under_a_held_migration_lock() {
    let harness = Harness::fresh("online_idx_missing_under_lock").await;
    migrate_profiled(&harness.pool)
        .await
        .expect("initial migration");
    sqlx::raw_sql("DROP INDEX idx_job_queue_created_id")
        .execute(&harness.pool)
        .await
        .expect("drop a declared index");
    let mut holder = sqlx::PgConnection::connect(harness.database.url())
        .await
        .expect("holder connection");
    holder.lock().await.expect("SQLx migration lock");

    let error = verify_profiled(&harness.pool)
        .await
        .expect_err("a missing index under a held migration lock is transient");
    assert!(
        matches!(
            error,
            SchemaCompatibilityError::OnlineIndex {
                index: "idx_job_queue_created_id",
                problem: OnlineIndexProblem::BuildInProgress
            }
        ),
        "unexpected error: {error}"
    );
    holder.unlock().await.expect("release SQLx lock");
    holder.close().await.expect("close holder");
    let error = verify_profiled(&harness.pool)
        .await
        .expect_err("without a holder the missing index is persistent");
    assert!(
        matches!(
            error,
            SchemaCompatibilityError::OnlineIndex {
                index: "idx_job_queue_created_id",
                problem: OnlineIndexProblem::Missing
            }
        ),
        "unexpected error: {error}"
    );
    harness.teardown().await;
}

#[tokio::test]
async fn lock_wait_honors_the_profile_statement_timeout_when_lock_timeout_is_zero() {
    let harness = Harness::fresh("online_idx_statement_timeout").await;
    raw_apply_through(&harness.pool, LAST_VERSION_BEFORE_SUMMARY_PAGINATION).await;
    let mut holder = sqlx::PgConnection::connect(harness.database.url())
        .await
        .expect("holder connection");
    holder.lock().await.expect("SQLx migration lock");

    let runner_pool = harness.pool.clone();
    let started = tokio::time::Instant::now();
    let error = tokio::time::timeout(
        Duration::from_secs(10),
        tokio::spawn(async move {
            migrate_with_profile_timeouts(&runner_pool, Duration::from_millis(500), Duration::ZERO)
                .await
        }),
    )
    .await
    .expect("the statement_timeout must bound the wait when lock_timeout is zero")
    .expect("runner task")
    .expect_err("a 500 ms statement_timeout bounds the wait for a held migration lock");
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "expiry must follow the profile bound, not the test deadline"
    );
    assert!(
        matches!(error, SchemaCompatibilityError::Incompatible(_)),
        "unexpected error: {error}"
    );
    assert!(
        error.to_string().contains("statement_timeout"),
        "unexpected message: {error}"
    );

    holder.unlock().await.expect("release SQLx lock");
    holder.close().await.expect("close holder");
    migrate_profiled(&harness.pool)
        .await
        .expect("startup proceeds once the lock is free");
    harness.teardown().await;
}

#[tokio::test]
async fn protected_runner_refuses_an_invalid_same_named_index_with_a_different_definition() {
    let harness = Harness::fresh("online_idx_invalid_foreign").await;
    migrate_profiled(&harness.pool)
        .await
        .expect("initial migration");
    sqlx::raw_sql("DROP INDEX idx_job_queue_created_id")
        .execute(&harness.pool)
        .await
        .expect("drop the declared index");
    // Leave an INVALID index of a different definition under the declared
    // name: a concurrent build cancelled while it waits for an open writer.
    let holder = hold_writer_lock(&harness.pool).await;
    let mut builder = harness.pool.acquire().await.expect("builder connection");
    sqlx::raw_sql("SET statement_timeout = '300ms'")
        .execute(&mut *builder)
        .await
        .expect("bound the build");
    sqlx::raw_sql("CREATE INDEX CONCURRENTLY idx_job_queue_created_id ON job_queue (id)")
        .execute(&mut *builder)
        .await
        .expect_err("the foreign build is cancelled by statement_timeout");
    drop(builder);
    holder.rollback().await.expect("release writer lock");
    assert!(
        matches!(
            index_catalog(&harness.pool)
                .await
                .get("idx_job_queue_created_id"),
            Some((_, false))
        ),
        "the cancelled foreign build leaves an INVALID index"
    );

    for (attempt, error) in [
        ("verification", verify_profiled(&harness.pool).await.err()),
        (
            "protected startup",
            migrate_profiled(&harness.pool).await.err(),
        ),
    ] {
        let error =
            error.unwrap_or_else(|| panic!("{attempt} must refuse the foreign INVALID index"));
        assert!(
            matches!(
                error,
                SchemaCompatibilityError::OnlineIndex {
                    index: "idx_job_queue_created_id",
                    problem: OnlineIndexProblem::DefinitionMismatch
                }
            ),
            "{attempt}: unexpected error: {error}"
        );
    }
    assert!(
        index_catalog(&harness.pool)
            .await
            .contains_key("idx_job_queue_created_id"),
        "a foreign INVALID index is refused, never dropped"
    );

    sqlx::raw_sql("DROP INDEX idx_job_queue_created_id")
        .execute(&harness.pool)
        .await
        .expect("remove the foreign index");
    migrate_profiled(&harness.pool)
        .await
        .expect("startup rebuilds the declared index once the name is free");
    assert_declared_indexes_valid(&harness.pool, "after foreign INVALID index removal").await;
    harness.teardown().await;
}
