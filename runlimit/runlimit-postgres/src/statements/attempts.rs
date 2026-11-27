//! The attempt limiter's statements.

statements! {
    /// Takes the shard's admission lock: `ATTEMPT_LOCK_NAMESPACE`, then the shard.
    SHARD_LOCK_SQL = "SELECT pg_catalog.pg_advisory_xact_lock($1, $2)";

    /// Reclaims up to 16 quiet subjects of one shard.
    CLEANUP_SQL = include_str!("../attempt_cleanup_qualified.sql");

    /// Locks the subject's row and reads exactly the columns `StoredAttempt::read` decodes.
    ROW_LOCK_SQL = "SELECT failures, last_failure_ms, retry_at_ms, \
     lease_until_ms FROM runlimit_attempts WHERE config_fingerprint = $1 AND subject_key = $2 \
     FOR UPDATE";

    /// Samples the database clock, in whole milliseconds, after the row lock.
    SAMPLE_TIME_SQL = "SELECT pg_catalog.floor(extract(epoch FROM \
     pg_catalog.clock_timestamp()) * 1000)::bigint AS now_ms";

    /// Counts every retained row of one shard, whatever slot bound wrote it.
    OCCUPANCY_SQL = "SELECT pg_catalog.count(*) AS row_count FROM runlimit_attempts WHERE capacity_shard = $1";

    /// Finds the lowest free slot below the configured bound, if any.
    FREE_SLOT_SQL = "SELECT pg_catalog.min(slot)::integer AS slot \
     FROM pg_catalog.generate_series(0, $2::integer - 1) AS slot \
     WHERE NOT EXISTS (SELECT 1 FROM runlimit_attempts \
     WHERE capacity_shard = $1 AND capacity_slot = slot)";

    /// Stores a new subject with its first lease in the slot found above.
    INSERT_SQL = "INSERT INTO runlimit_attempts \
     (config_fingerprint, subject_key, capacity_slot, failures, last_failure_ms, retry_at_ms, \
     quiet_ms, lease_until_ms, lease_token) \
     VALUES ($1, $2, $3, $4, $5, $5, $6, $7, pg_catalog.gen_random_uuid()::pg_catalog.text) \
     RETURNING lease_token";

    /// Grants a stored subject a new lease.
    LEASE_SQL = "UPDATE runlimit_attempts \
     SET failures = $3, last_failure_ms = $4, retry_at_ms = $4, lease_until_ms = $5, \
     lease_token = pg_catalog.gen_random_uuid()::pg_catalog.text \
     WHERE config_fingerprint = $1 AND subject_key = $2 RETURNING lease_token";

    /// Rotates a live lease's token to the claiming transaction, after the row lock.
    CLAIM_SQL = "WITH locked AS MATERIALIZED (\
     SELECT config_fingerprint, subject_key, lease_token, lease_until_ms FROM runlimit_attempts \
     WHERE config_fingerprint = $1 AND subject_key = $2 FOR UPDATE), \
     sampled(config_fingerprint, subject_key, lease_token, lease_until_ms, now_ms) AS MATERIALIZED (\
     SELECT *, pg_catalog.floor(extract(epoch FROM \
     pg_catalog.clock_timestamp()) * 1000)::bigint AS now_ms FROM locked) \
     UPDATE runlimit_attempts AS attempts \
     SET lease_token = pg_catalog.gen_random_uuid()::pg_catalog.text FROM sampled \
     WHERE attempts.config_fingerprint = sampled.config_fingerprint \
     AND attempts.subject_key = sampled.subject_key AND sampled.lease_token = $3 \
     AND sampled.lease_until_ms > sampled.now_ms \
     RETURNING attempts.lease_token, \
     pg_catalog.pg_current_xact_id()::pg_catalog.text AS transaction_id";

    /// Applies a completion and its fence in one statement.
    COMPLETE_SQL = include_str!("../attempt_complete_scheduled.sql");
}
