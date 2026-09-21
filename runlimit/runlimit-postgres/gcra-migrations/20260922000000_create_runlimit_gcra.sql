-- GCRA v1 is an independent, opt-in storage and locking protocol.
-- All admission and cleanup transactions lock the affected shard-ledger rows
-- in ascending order BEFORE reading or mutating counters. The shard count,
-- XOR derivation, and hard ceiling are immutable cross-replica protocol.
CREATE TABLE runlimit_gcra_shards (
    capacity_shard SMALLINT PRIMARY KEY CHECK (capacity_shard BETWEEN 0 AND 255),
    row_count BIGINT NOT NULL DEFAULT 0 CHECK (row_count BETWEEN 0 AND 65536),
    observed_at_ms BIGINT NOT NULL DEFAULT 0 CHECK (observed_at_ms >= 0)
);

INSERT INTO runlimit_gcra_shards (capacity_shard)
SELECT shard::SMALLINT FROM pg_catalog.generate_series(0, 255) AS shards(shard);

CREATE TABLE runlimit_gcra (
    config_fingerprint BYTEA NOT NULL CHECK (octet_length(config_fingerprint) = 32),
    subject_key BYTEA NOT NULL CHECK (octet_length(subject_key) = 32),
    -- Exact integer arithmetic; PostgreSQL BIGINT cannot hold quota-scaled
    -- epoch milliseconds for the full validated policy range.
    tat_scaled NUMERIC(39, 0) NOT NULL CHECK (tat_scaled >= 0),
    expires_at_ms BIGINT NOT NULL CHECK (expires_at_ms >= 0),
    capacity_shard SMALLINT GENERATED ALWAYS AS (
        (pg_catalog.get_byte(config_fingerprint, 0)
            # pg_catalog.get_byte(subject_key, 0))::SMALLINT
    ) STORED NOT NULL REFERENCES runlimit_gcra_shards (capacity_shard),
    PRIMARY KEY (config_fingerprint, subject_key)
) WITH (fillfactor = 80);

CREATE INDEX runlimit_gcra_expiry ON runlimit_gcra (expires_at_ms, capacity_shard);

CREATE FUNCTION runlimit_gcra_capacity_change()
RETURNS TRIGGER LANGUAGE plpgsql SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $$
BEGIN
    IF TG_OP = 'UPDATE' THEN
        IF NEW.config_fingerprint IS DISTINCT FROM OLD.config_fingerprint
            OR NEW.subject_key IS DISTINCT FROM OLD.subject_key THEN
            RAISE EXCEPTION 'runlimit GCRA storage keys are immutable'
                USING ERRCODE = 'check_violation';
        END IF;
        RETURN NEW;
    ELSIF TG_OP = 'INSERT' THEN
        EXECUTE pg_catalog.format(
            'UPDATE %I.runlimit_gcra_shards SET row_count = row_count + 1 WHERE capacity_shard = $1',
            TG_TABLE_SCHEMA)
        USING NEW.capacity_shard;
        RETURN NEW;
    ELSE
        EXECUTE pg_catalog.format(
            'UPDATE %I.runlimit_gcra_shards SET row_count = row_count - 1 WHERE capacity_shard = $1',
            TG_TABLE_SCHEMA)
        USING OLD.capacity_shard;
        RETURN OLD;
    END IF;
END;
$$;

CREATE TRIGGER runlimit_gcra_capacity_insert_delete
AFTER INSERT OR DELETE ON runlimit_gcra
FOR EACH ROW EXECUTE FUNCTION runlimit_gcra_capacity_change();

CREATE TRIGGER runlimit_gcra_immutable_key
BEFORE UPDATE OF config_fingerprint, subject_key ON runlimit_gcra
FOR EACH ROW EXECUTE FUNCTION runlimit_gcra_capacity_change();
