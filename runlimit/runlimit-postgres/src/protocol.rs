use std::time::Duration;

use runlimit_core::CounterKey;
use sha2::{Digest, Sha256};
use sqlx::Error;
use tokio::time::Instant;

const ADVISORY_LOCK_DOMAIN: &[u8] = b"runlimit/postgres-advisory-lock/v1\0";
pub(crate) const SERVER_TIMEOUT_GRACE: Duration = Duration::from_millis(25);
/// Number of stable capacity shards used by `PostgreSQL` storage.
///
/// The shard derivation is a persistent cross-replica protocol and this value
/// must not change in place.
pub const CAPACITY_SHARD_COUNT: usize = 256;

/// Database-enforced maximum number of fixed-window rows in one capacity
/// shard.
///
/// The additive cardinality migration enforces this ceiling independently of
/// the runtime configuration, including for replicas running older code.
pub const HARD_MAX_ROWS_PER_SHARD: u32 = 65_536;

pub(crate) const SET_LOCAL_TIMEOUTS_SQL: &str = r"
SELECT
    set_config('statement_timeout', $1, true),
    set_config('lock_timeout', $2, true)
";

pub(crate) const BATCH_ADVISORY_LOCK_SQL: &str = r"
WITH RECURSIVE acquired(position, locked) AS (
    SELECT 1, pg_advisory_xact_lock($1[1])
    WHERE cardinality($1::BIGINT[]) > 0

    UNION ALL

    SELECT
        acquired.position + 1,
        pg_advisory_xact_lock($1[acquired.position + 1])
    FROM acquired
    WHERE acquired.position < cardinality($1::BIGINT[])
)
SELECT count(*)
FROM acquired
";

pub(crate) const BATCH_ROW_LOCK_SQL: &str = r"
WITH input_keys AS (
    SELECT *
    FROM unnest(
        $1::BYTEA[],
        $2::BYTEA[]
    ) WITH ORDINALITY AS keys(
        config_fingerprint,
        subject_key,
        input_position
    )
),
lock_order AS (
    SELECT *
    FROM unnest($3::BIGINT[])
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

pub(crate) const BATCH_CAPACITY_LOCK_SQL: &str = r"
WITH input_keys AS (
    SELECT *
    FROM unnest(
        $1::BYTEA[],
        $2::BYTEA[],
        $3::SMALLINT[]
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
    SELECT count(*) AS locked_count
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

pub(crate) const BATCH_PREFLIGHT_SQL: &str = r"
WITH input AS (
    SELECT *
    FROM unnest(
        $1::BYTEA[],
        $2::BYTEA[],
        $3::BIGINT[],
        $4::BIGINT[]
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

pub(crate) const BATCH_UPSERT_SQL: &str = r"
WITH input AS (
    SELECT *
    FROM unnest(
        $1::TEXT[],
        $2::TEXT[],
        $3::BYTEA[],
        $4::BYTEA[],
        $5::INTERVAL[],
        $6::BIGINT[],
        $7::BIGINT[]
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
    HAVING count(*) >= 0
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

pub(crate) const CLEANUP_SQL: &str = include_str!("cleanup_expired.sql");

pub(crate) fn is_server_timeout(error: &Error) -> bool {
    let sqlx::Error::Database(database_error) = error else {
        return false;
    };
    matches!(database_error.code().as_deref(), Some("57014" | "55P03"))
}

pub(crate) fn remaining_server_timeout_settings(deadline: Instant) -> Option<(String, String)> {
    let remaining = deadline.checked_duration_since(Instant::now())?;
    let remaining_millis = remaining.as_millis();
    let statement_millis = remaining_millis
        .saturating_sub(SERVER_TIMEOUT_GRACE.as_millis())
        .max(1);
    let lock_millis = statement_millis.saturating_sub(1).max(1);
    Some((format!("{statement_millis}ms"), format!("{lock_millis}ms")))
}

pub(crate) fn advisory_lock_id(counter_key: CounterKey) -> i64 {
    let mut digest = Sha256::new();
    digest.update(ADVISORY_LOCK_DOMAIN);
    digest.update(counter_key.to_bytes());
    let digest = digest.finalize();
    let mut id_bytes = [0_u8; 8];
    let id_length = id_bytes.len();
    id_bytes.copy_from_slice(&digest[..id_length]);
    i64::from_be_bytes(id_bytes)
}

pub(crate) fn capacity_shard(counter_key: CounterKey) -> i16 {
    i16::from(counter_key.fingerprint().as_bytes()[0] ^ counter_key.subject().as_bytes()[0])
}
