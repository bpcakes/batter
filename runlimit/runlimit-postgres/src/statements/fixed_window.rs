//! The fixed-window limiter's statements.

statements! {
    BATCH_ADVISORY_LOCK_SQL = r"
WITH RECURSIVE acquired(position, locked) AS (
    SELECT 1, pg_catalog.pg_advisory_xact_lock($1[1])
    WHERE pg_catalog.cardinality($1::BIGINT[]) > 0

    UNION ALL

    SELECT
        acquired.position + 1,
        pg_catalog.pg_advisory_xact_lock($1[acquired.position + 1])
    FROM acquired
    WHERE acquired.position < pg_catalog.cardinality($1::BIGINT[])
)
SELECT pg_catalog.count(*)
FROM acquired
";

    BATCH_ROW_LOCK_SQL = r"
WITH input_keys AS (
    SELECT *
    FROM ROWS FROM (
        pg_catalog.unnest($1::pg_catalog.bytea[]),
        pg_catalog.unnest($2::pg_catalog.bytea[])
    ) WITH ORDINALITY AS keys(
        config_fingerprint,
        subject_key,
        input_position
    )
),
lock_order AS (
    SELECT *
    FROM pg_catalog.unnest($3::BIGINT[])
        WITH ORDINALITY AS positions(input_position, lock_position)
),
ordered_keys AS (
    SELECT
        input_keys.config_fingerprint,
        input_keys.subject_key,
        lock_order.lock_position
    FROM input_keys
    INNER JOIN lock_order
        ON lock_order.input_position = input_keys.input_position
    ORDER BY lock_order.lock_position
)
SELECT 1
FROM ordered_keys
INNER JOIN runlimit_fixed_windows AS windows
    ON windows.config_fingerprint = ordered_keys.config_fingerprint
    AND windows.subject_key = ordered_keys.subject_key
ORDER BY ordered_keys.lock_position
FOR UPDATE OF windows
";

    BATCH_CAPACITY_LOCK_SQL = r"
WITH input_keys AS (
    SELECT *
    FROM ROWS FROM (
        pg_catalog.unnest($1::pg_catalog.bytea[]),
        pg_catalog.unnest($2::pg_catalog.bytea[]),
        pg_catalog.unnest($3::SMALLINT[])
    ) WITH ORDINALITY AS keys(
        config_fingerprint,
        subject_key,
        capacity_shard,
        input_position
    )
),
missing_keys AS MATERIALIZED (
    SELECT
        input_keys.capacity_shard,
        input_keys.input_position
    FROM input_keys
    LEFT JOIN runlimit_fixed_windows AS windows
        ON windows.config_fingerprint = input_keys.config_fingerprint
        AND windows.subject_key = input_keys.subject_key
    WHERE windows.config_fingerprint IS NULL
),
target_shards AS (
    SELECT DISTINCT capacity_shard
    FROM missing_keys
),
locked_shards AS MATERIALIZED (
    SELECT
        capacity.capacity_shard,
        capacity.row_count
    FROM runlimit_capacity_shards AS capacity
    INNER JOIN target_shards
        ON target_shards.capacity_shard = capacity.capacity_shard
    ORDER BY capacity.capacity_shard
    FOR UPDATE OF capacity
),
lock_barrier AS MATERIALIZED (
    SELECT pg_catalog.count(*) AS locked_count
    FROM locked_shards
)
SELECT
    missing_keys.input_position - 1 AS input_index,
    missing_keys.capacity_shard,
    locked_shards.row_count
FROM missing_keys
LEFT JOIN locked_shards
    ON locked_shards.capacity_shard = missing_keys.capacity_shard
CROSS JOIN lock_barrier
ORDER BY missing_keys.input_position
";

    BATCH_PREFLIGHT_SQL = r"
WITH input AS (
    SELECT *
    FROM ROWS FROM (
        pg_catalog.unnest($1::pg_catalog.bytea[]),
        pg_catalog.unnest($2::pg_catalog.bytea[]),
        pg_catalog.unnest($3::BIGINT[]),
        pg_catalog.unnest($4::BIGINT[])
    ) WITH ORDINALITY AS checks(
        config_fingerprint,
        subject_key,
        cost,
        quota_limit,
        input_position
    )
),
sample AS (
    SELECT pg_catalog.clock_timestamp() AS database_now
),
first_denial AS (
    SELECT
        input.input_position - 1 AS input_index,
        windows.window_expires_at
    FROM input
    CROSS JOIN sample
    INNER JOIN runlimit_fixed_windows AS windows
        ON windows.config_fingerprint = input.config_fingerprint
        AND windows.subject_key = input.subject_key
    WHERE
        windows.window_expires_at > sample.database_now
        AND windows.used > input.quota_limit - input.cost
    ORDER BY input.input_position
    LIMIT 1
)
SELECT
    sample.database_now,
    pg_catalog.clock_timestamp() AS response_now,
    first_denial.input_index,
    first_denial.window_expires_at
FROM sample
LEFT JOIN first_denial ON TRUE
";

    BATCH_UPSERT_SQL = r"
WITH input AS (
    SELECT *
    FROM ROWS FROM (
        pg_catalog.unnest($1::pg_catalog.text[]),
        pg_catalog.unnest($2::pg_catalog.text[]),
        pg_catalog.unnest($3::pg_catalog.bytea[]),
        pg_catalog.unnest($4::pg_catalog.bytea[]),
        pg_catalog.unnest($5::INTERVAL[]),
        pg_catalog.unnest($6::BIGINT[]),
        pg_catalog.unnest($7::BIGINT[])
    ) WITH ORDINALITY AS checks(
        policy_id,
        scope_id,
        config_fingerprint,
        subject_key,
        window_size,
        cost,
        quota_limit,
        input_position
    )
),
upserted AS (
    INSERT INTO runlimit_fixed_windows (
        policy_id,
        scope_id,
        config_fingerprint,
        subject_key,
        window_started_at,
        window_expires_at,
        used
    )
    SELECT
        policy_id,
        scope_id,
        config_fingerprint,
        subject_key,
        $8,
        $8 + window_size,
        cost
    FROM input
    ON CONFLICT (config_fingerprint, subject_key)
    DO UPDATE SET
        policy_id = EXCLUDED.policy_id,
        scope_id = EXCLUDED.scope_id,
        window_started_at = CASE
            WHEN runlimit_fixed_windows.window_expires_at <= $8 THEN $8
            ELSE runlimit_fixed_windows.window_started_at
        END,
        window_expires_at = CASE
            WHEN runlimit_fixed_windows.window_expires_at <= $8
                THEN $8 + (EXCLUDED.window_expires_at - EXCLUDED.window_started_at)
            ELSE runlimit_fixed_windows.window_expires_at
        END,
        used = CASE
            WHEN runlimit_fixed_windows.window_expires_at <= $8 THEN EXCLUDED.used
            ELSE runlimit_fixed_windows.used + EXCLUDED.used
        END
    WHERE
        runlimit_fixed_windows.window_expires_at <= $8
        OR runlimit_fixed_windows.used
            <= (
                SELECT quota_limit - cost
                FROM input
                WHERE
                    input.config_fingerprint = EXCLUDED.config_fingerprint
                    AND input.subject_key = EXCLUDED.subject_key
            )
    RETURNING
        config_fingerprint,
        subject_key,
        used,
        window_expires_at
),
response AS (
    SELECT pg_catalog.clock_timestamp() AS response_now
    FROM upserted
    HAVING pg_catalog.count(*) >= 0
)
SELECT
    input.input_position - 1 AS input_index,
    upserted.used,
    upserted.window_expires_at,
    response.response_now
FROM input
INNER JOIN upserted
    ON upserted.config_fingerprint = input.config_fingerprint
    AND upserted.subject_key = input.subject_key
CROSS JOIN response
ORDER BY input.input_position
";

    CLEANUP_SQL = include_str!("../cleanup_expired_by_key.sql");
}
