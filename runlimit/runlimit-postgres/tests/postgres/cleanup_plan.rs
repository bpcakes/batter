//! Access-path evidence for the executed cleanup statement, including plan reuse.

use sqlx::{AssertSqlSafe, PgConnection, PgPool};

use super::CLEANUP_SQL;

pub(super) async fn assert_indexed_cleanup(pool: &PgPool) {
    let mut transaction = pool.begin().await.expect("begin plan transaction");
    // Populate enough live storage to make a full-table delete join materially
    // different from probing the one selected victim. Roll this back before the
    // caller's row-lock and deletion assertions.
    sqlx::query(
        r"
INSERT INTO runlimit_fixed_windows (
    policy_id, scope_id, config_fingerprint, subject_key,
    window_started_at, window_expires_at, used
)
SELECT
    'cleanup-plan', 'global', decode(repeat('01', 32), 'hex'),
    decode(lpad(to_hex(i), 64, '0'), 'hex'),
    clock_timestamp() - INTERVAL '2 hours',
    clock_timestamp() + CASE WHEN i <= 20
        THEN INTERVAL '-1 hour' ELSE INTERVAL '1 hour' END,
    1
FROM generate_series(1, 20000) AS i
",
    )
    .execute(&mut *transaction)
    .await
    .expect("populate cleanup plan fixture");
    sqlx::raw_sql(
        "ANALYZE runlimit_fixed_windows;
         SET LOCAL plan_cache_mode = auto;
         SET LOCAL enable_seqscan = on;",
    )
    .execute(&mut *transaction)
    .await
    .expect("collect statistics and use automatic plan selection");
    sqlx::raw_sql(AssertSqlSafe(format!(
        "PREPARE cleanup_plan(bigint) AS {CLEANUP_SQL}"
    )))
    .execute(&mut *transaction)
    .await
    .expect("prepare the production cleanup statement");

    // The sixth execution reaches PostgreSQL's generic/custom-plan comparison.
    // Do not disable sequential scans to make these assertions pass.
    for execution in 1..=6 {
        let plan = explain(&mut transaction).await;
        assert!(
            plan.contains("Index Scan using runlimit_fixed_windows_expiry_idx")
                && plan.lines().any(|line| {
                    line.contains("Index Cond:") && line.contains("window_expires_at")
                }),
            "execution {execution} lost the indexed expiry cutoff:\n{plan}"
        );
        assert!(
            probes_selected_keys(&plan),
            "execution {execution} scans instead of probing the selected delete keys:\n{plan}"
        );
    }

    assert_scan_controls(&mut transaction).await;
    sqlx::query("DEALLOCATE cleanup_plan")
        .execute(&mut *transaction)
        .await
        .expect("remove the session's prepared statement");
    transaction.rollback().await.expect("remove plan fixture");
}

async fn assert_scan_controls(connection: &mut PgConnection) {
    // A forced generic plan can estimate far more victims than LIMIT 1. Inspect
    // that real plan too; bounded deletions do not promise bounded planner cost.
    sqlx::query("SET LOCAL plan_cache_mode = force_generic_plan")
        .execute(&mut *connection)
        .await
        .expect("force generic prepared-plan selection");
    let generic = explain(connection).await;
    let generic_plans: i64 = sqlx::query_scalar(
        "SELECT generic_plans FROM pg_prepared_statements WHERE name = 'cleanup_plan'",
    )
    .fetch_one(&mut *connection)
    .await
    .expect("verify a generic plan was actually selected");
    assert!(generic_plans > 0, "generic plan was not exercised");
    if generic.contains("Seq Scan on runlimit_fixed_windows windows") {
        assert!(
            !probes_selected_keys(&generic),
            "the access-path guard accepted a generic full-table delete scan:\n{generic}"
        );
    }

    // Independently force a scan as a negative control, even if a future server
    // improves generic planning. Re-prepare after changing planner settings.
    sqlx::raw_sql(
        "DEALLOCATE cleanup_plan;
         SET LOCAL enable_indexscan = off;
         SET LOCAL enable_indexonlyscan = off;
         SET LOCAL enable_bitmapscan = off;",
    )
    .execute(&mut *connection)
    .await
    .expect("prepare a full-scan negative control");
    sqlx::raw_sql(AssertSqlSafe(format!(
        "PREPARE cleanup_plan(bigint) AS {CLEANUP_SQL}"
    )))
    .execute(&mut *connection)
    .await
    .expect("re-prepare without indexed access paths");
    let scan = explain(connection).await;
    assert!(scan.contains("Seq Scan on runlimit_fixed_windows windows"));
    assert!(
        !probes_selected_keys(&scan),
        "the access-path guard accepted the forced full-table delete scan:\n{scan}"
    );
}

async fn explain(connection: &mut PgConnection) -> String {
    sqlx::query_scalar::<_, String>("EXPLAIN (COSTS OFF) EXECUTE cleanup_plan(1)")
        .fetch_all(connection)
        .await
        .expect("explain prepared production cleanup")
        .join("\n")
}

fn probes_selected_keys(plan: &str) -> bool {
    let mut lines = plan.lines();
    while let Some(line) = lines.next() {
        if line.contains(
            "Index Scan using runlimit_fixed_windows_pkey on runlimit_fixed_windows windows",
        ) {
            return lines.next().is_some_and(|condition| {
                condition.contains("Index Cond:")
                    && condition.contains("config_fingerprint = expired.config_fingerprint")
                    && condition.contains("subject_key = expired.subject_key")
            });
        }
    }
    false
}
