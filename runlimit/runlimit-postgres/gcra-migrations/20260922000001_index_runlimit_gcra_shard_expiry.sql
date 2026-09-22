-- Cleanup probes each bounded capacity shard using its own monotonic clock.
-- Keep the original global expiry index for replicas using the older query.
CREATE INDEX runlimit_gcra_shard_expiry
    ON runlimit_gcra (capacity_shard, expires_at_ms);
