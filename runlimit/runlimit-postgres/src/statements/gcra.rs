//! The GCRA limiter's statements.

statements! {
    LOCK_SHARDS = "SELECT capacity_shard, row_count, observed_at_ms FROM runlimit_gcra_shards WHERE capacity_shard = ANY($1) ORDER BY capacity_shard FOR UPDATE";

    SAMPLE_TIME = r"
WITH sample AS MATERIALIZED (SELECT pg_catalog.clock_timestamp() AS database_now)
SELECT database_now, pg_catalog.floor(extract(epoch FROM database_now) * 1000)::BIGINT AS now_ms
FROM sample";

    READ_COUNTERS = r"
SELECT input.ordinality - 1 AS input_index, stored.tat_scaled::pg_catalog.text AS tat_scaled,
    stored.expires_at_ms, pg_catalog.clock_timestamp() AS response_now
FROM ROWS FROM (pg_catalog.unnest($1::pg_catalog.bytea[]), pg_catalog.unnest($2::pg_catalog.bytea[])) WITH ORDINALITY AS input(fingerprint, subject, ordinality)
LEFT JOIN runlimit_gcra AS stored ON stored.config_fingerprint = input.fingerprint
    AND stored.subject_key = input.subject
ORDER BY input.ordinality";

    UPSERT = r"
INSERT INTO runlimit_gcra(config_fingerprint, subject_key, tat_scaled, expires_at_ms)
SELECT fingerprint, subject, tat::NUMERIC, expires
FROM ROWS FROM (pg_catalog.unnest($1::pg_catalog.bytea[]), pg_catalog.unnest($2::pg_catalog.bytea[]), pg_catalog.unnest($3::pg_catalog.text[]), pg_catalog.unnest($4::BIGINT[])) AS input(fingerprint, subject, tat, expires)
ON CONFLICT (config_fingerprint, subject_key) DO UPDATE
SET tat_scaled = EXCLUDED.tat_scaled, expires_at_ms = EXCLUDED.expires_at_ms";

    UPDATE_WATERMARKS = r"
WITH updated AS (
    UPDATE runlimit_gcra_shards SET observed_at_ms = $2 WHERE capacity_shard = ANY($1)
    RETURNING 1
)
SELECT pg_catalog.count(*) AS updated_count, pg_catalog.clock_timestamp() AS response_now FROM updated";

    CLEANUP_LOCKS = r"
WITH sample AS MATERIALIZED (
    SELECT pg_catalog.floor(extract(epoch FROM pg_catalog.clock_timestamp()) * 1000)::BIGINT AS now_ms
), candidates AS MATERIALIZED (
    -- At most 256 indexed probes, each returning at most one candidate. Using
    -- the same shard clock as admission makes logically expired slots
    -- reclaimable even while the physical clock remains behind its watermark.
    SELECT shard.capacity_shard FROM runlimit_gcra_shards AS shard
    CROSS JOIN LATERAL (
        SELECT 1 FROM runlimit_gcra AS counter
        WHERE counter.capacity_shard = shard.capacity_shard
            AND counter.expires_at_ms <= greatest((SELECT now_ms FROM sample), shard.observed_at_ms)
        ORDER BY counter.expires_at_ms LIMIT 1
    ) AS expired
)
SELECT capacity.capacity_shard FROM runlimit_gcra_shards AS capacity
WHERE capacity.capacity_shard IN (SELECT capacity_shard FROM candidates)
ORDER BY capacity.capacity_shard FOR UPDATE OF capacity SKIP LOCKED";

    CLEANUP_ADVANCE_TIME = r"
WITH sample AS MATERIALIZED (
    SELECT pg_catalog.floor(extract(epoch FROM pg_catalog.clock_timestamp()) * 1000)::BIGINT AS now_ms
)
UPDATE runlimit_gcra_shards
SET observed_at_ms = greatest((SELECT now_ms FROM sample), observed_at_ms)
WHERE capacity_shard = ANY($1)";

    CLEANUP_DELETE = r"
WITH expired AS MATERIALIZED (
    SELECT candidate.config_fingerprint, candidate.subject_key
    FROM runlimit_gcra_shards AS shard
    CROSS JOIN LATERAL (
        SELECT config_fingerprint, subject_key, expires_at_ms FROM runlimit_gcra AS counter
        WHERE counter.capacity_shard = shard.capacity_shard
            AND counter.expires_at_ms <= shard.observed_at_ms
        ORDER BY counter.expires_at_ms LIMIT $2
    ) AS candidate
    WHERE shard.capacity_shard = ANY($1)
    ORDER BY candidate.expires_at_ms LIMIT $2
)
DELETE FROM runlimit_gcra AS stored USING expired
WHERE stored.config_fingerprint = expired.config_fingerprint AND stored.subject_key = expired.subject_key";
}
