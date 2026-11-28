WITH authoritative_time AS MATERIALIZED (
    SELECT pg_catalog.clock_timestamp() AS sampled_at
),
expired AS MATERIALIZED (
    SELECT
        ctid,
        capacity_shard
    FROM runlimit_fixed_windows
    -- The scalar subquery becomes an init-plan parameter, allowing the expiry
    -- index to stop before active windows instead of filtering its full scan.
    WHERE window_expires_at <= (
        SELECT sampled_at
        FROM authoritative_time
    )
    ORDER BY window_expires_at
    FOR UPDATE SKIP LOCKED
    LIMIT $1
),
target_shards AS MATERIALIZED (
    SELECT DISTINCT capacity_shard
    FROM expired
),
locked_capacity AS MATERIALIZED (
    SELECT capacity.capacity_shard
    FROM runlimit_capacity_shards AS capacity
    INNER JOIN target_shards
        ON target_shards.capacity_shard = capacity.capacity_shard
    ORDER BY capacity.capacity_shard
    FOR UPDATE OF capacity
),
capacity_lock_barrier AS MATERIALIZED (
    SELECT count(*) AS locked_count
    FROM locked_capacity
)
DELETE FROM runlimit_fixed_windows AS windows
USING expired, capacity_lock_barrier
WHERE
    windows.ctid = expired.ctid
    AND capacity_lock_barrier.locked_count = (
        SELECT count(*)
        FROM target_shards
    )
