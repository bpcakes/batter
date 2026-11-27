-- The materialized locked row is consumed before sampling time. A statement
-- blocked on the row cannot validate a lease using its stale pre-lock clock.
-- This applies the live-lease case of runlimit_core's AttemptState::complete.
-- $5 is AttemptPolicy::delay_schedule in milliseconds: element n is the delay
-- after n consecutive failures and every later count waits the last element.
-- The schedule is derived in Rust only; do not restate its formula here.
-- A stored count below zero fits no state. The lookup stays in bounds for it
-- so the returned count, not a NULL delay, reaches the caller's validation.
WITH locked AS MATERIALIZED (
    SELECT * FROM runlimit_attempts
    WHERE config_fingerprint = $1 AND subject_key = $2
    FOR UPDATE
), sampled AS MATERIALIZED (
    SELECT locked.*, floor(extract(epoch FROM pg_catalog.clock_timestamp()) * 1000)::bigint AS now_ms
    FROM locked
), next AS MATERIALIZED (
    SELECT *, CASE WHEN $4 THEN 0 ELSE LEAST(failures + 1, 4294967295) END AS next_failures,
        CASE WHEN $4 THEN 0 ELSE
            ($5::bigint[])[GREATEST(
                LEAST(failures + 1, pg_catalog.cardinality($5::bigint[])), 1
            )::integer]
        END AS delay_ms
    FROM sampled WHERE lease_token = $3 AND (
        ($6::text IS NULL AND lease_until_ms > now_ms)
        OR $6 = pg_catalog.pg_current_xact_id()::text
    )
)
UPDATE runlimit_attempts AS attempts
SET failures = next.next_failures, last_failure_ms = next.now_ms,
    retry_at_ms = next.now_ms + next.delay_ms, lease_token = NULL, lease_until_ms = NULL
FROM next
WHERE attempts.config_fingerprint = next.config_fingerprint AND attempts.subject_key = next.subject_key
RETURNING attempts.failures, next.delay_ms
