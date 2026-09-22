use super::Database;
use sqlx::AssertSqlSafe;

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn dense_shard_cleanup_uses_expiry_as_an_index_condition() {
    let db = Database::new().await;
    // Fill one shard to the database ceiling, with both idle and leased rows.
    // Every row remains active for an hour; no wall-clock latency threshold is
    // needed to distinguish an indexed cutoff from filtering all 65,536 rows.
    sqlx::query(
        "WITH clock AS MATERIALIZED (
            SELECT floor(extract(epoch FROM clock_timestamp()) * 1000)::bigint
                + 3600000 AS future_ms
        )
        INSERT INTO runlimit_attempts (
            config_fingerprint, subject_key, capacity_slot, failures,
            last_failure_ms, retry_at_ms, quiet_ms, lease_until_ms, lease_token
        )
        SELECT decode(repeat('00', 32), 'hex'),
            decode(lpad(to_hex(slot), 64, '0'), 'hex'), slot, 1,
            future_ms, future_ms, 60000,
            CASE WHEN slot % 2 = 0 THEN future_ms END,
            CASE WHEN slot % 2 = 0 THEN 'synthetic-lease' END
        FROM generate_series(0, 65535) AS slots(slot), clock",
    )
    .execute(&db.pool)
    .await
    .unwrap();

    for expired in [0_i64, 3] {
        sqlx::query(
            "UPDATE runlimit_attempts SET last_failure_ms=0,
                lease_until_ms=CASE WHEN lease_token IS NOT NULL THEN 0 END
             WHERE capacity_slot < $1",
        )
        .bind(expired)
        .execute(&db.pool)
        .await
        .unwrap();
        sqlx::query("ANALYZE runlimit_attempts")
            .execute(&db.pool)
            .await
            .unwrap();

        let mut tx = db.pool.begin().await.unwrap();
        sqlx::query("SELECT pg_advisory_xact_lock($1, $2)")
            .bind(0x524c_4154_i32)
            .bind(0_i32)
            .execute(&mut *tx)
            .await
            .unwrap();
        // Explain and execute the exact production DELETE with normal planner
        // settings, not a copied candidate SELECT or a forced index scan.
        let plan = sqlx::query_scalar::<_, String>(AssertSqlSafe(format!(
            "EXPLAIN (ANALYZE, BUFFERS, COSTS OFF, TIMING OFF) {}",
            include_str!("../../src/attempt_cleanup.sql")
        )))
        .bind(0_i16)
        .fetch_all(&mut *tx)
        .await
        .unwrap()
        .join("\n");
        println!("attempt cleanup plan ({expired} expired):\n{plan}");
        assert!(
            plan.contains("Index Scan using runlimit_attempts_expiry"),
            "cleanup must use the shard expiry index ({expired} expired):\n{plan}"
        );
        assert!(
            plan.lines().any(|line| {
                line.contains("Index Cond:")
                    && line.contains("capacity_shard")
                    && line.contains("COALESCE(lease_until_ms, last_failure_ms)")
                    && line.contains("quiet_ms")
                    && line.contains("<=")
            }),
            "expiry must bound the index scan ({expired} expired):\n{plan}"
        );
        assert!(
            !plan.contains("Rows Removed by Filter:"),
            "cleanup must not filter active entries ({expired} expired):\n{plan}"
        );
        assert!(
            plan.contains("Buffers:"),
            "missing buffer evidence:\n{plan}"
        );
        let retained: i64 = sqlx::query_scalar("SELECT count(*) FROM runlimit_attempts")
            .fetch_one(&mut *tx)
            .await
            .unwrap();
        assert_eq!(retained, 65536 - expired);
        tx.rollback().await.unwrap();
    }
    db.close().await;
}
