WITH authoritative_time AS MATERIALIZED (
    SELECT floor(extract(epoch FROM pg_catalog.clock_timestamp()) * 1000)::bigint AS now_ms
)
DELETE FROM runlimit_attempts
WHERE (config_fingerprint, subject_key) IN (
    SELECT config_fingerprint, subject_key
    FROM runlimit_attempts
    WHERE capacity_shard = $1
        -- The scalar subquery becomes an init-plan parameter, allowing the
        -- expiry index to stop before active entries instead of filtering them.
        -- This statement runs after the shard lock; admission samples time
        -- separately after acquiring the subject row lock.
        AND COALESCE(lease_until_ms, last_failure_ms) + quiet_ms
            <= (SELECT now_ms FROM authoritative_time)
    ORDER BY COALESCE(lease_until_ms, last_failure_ms) + quiet_ms
    LIMIT 16
    FOR UPDATE SKIP LOCKED
)
