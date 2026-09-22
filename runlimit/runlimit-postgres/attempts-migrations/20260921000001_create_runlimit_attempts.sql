-- New protocol; existing quota storage/migrations are unchanged.
-- Slot uniqueness plus the fixed slot range enforce a hard capacity even for
-- writers that do not follow the library's advisory-lock admission protocol.
CREATE TABLE runlimit_attempts (
    config_fingerprint BYTEA NOT NULL CHECK (octet_length(config_fingerprint) = 32),
    subject_key BYTEA NOT NULL CHECK (octet_length(subject_key) = 32),
    capacity_shard SMALLINT GENERATED ALWAYS AS
        ((get_byte(config_fingerprint, 0) # get_byte(subject_key, 0))::SMALLINT) STORED,
    capacity_slot INTEGER NOT NULL CHECK (capacity_slot BETWEEN 0 AND 65535),
    failures BIGINT NOT NULL CHECK (failures BETWEEN 0 AND 4294967295),
    last_failure_ms BIGINT NOT NULL,
    retry_at_ms BIGINT NOT NULL,
    quiet_ms BIGINT NOT NULL CHECK (quiet_ms BETWEEN 1 AND 9007199254740),
    lease_until_ms BIGINT,
    lease_token TEXT,
    PRIMARY KEY (config_fingerprint, subject_key),
    UNIQUE (capacity_shard, capacity_slot),
    CHECK ((lease_until_ms IS NULL) = (lease_token IS NULL))
);
CREATE INDEX runlimit_attempts_expiry ON runlimit_attempts
    (capacity_shard, ((COALESCE(lease_until_ms, last_failure_ms)) + quiet_ms));
