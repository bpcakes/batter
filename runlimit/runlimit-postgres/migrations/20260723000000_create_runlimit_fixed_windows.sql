-- Runlimit's fixed windows are anchored to the first admitted check. PostgreSQL
-- is the time authority; applications must not write this table directly.
CREATE TABLE runlimit_fixed_windows (
    policy_id TEXT NOT NULL,
    scope_id TEXT NOT NULL,
    config_fingerprint BYTEA NOT NULL,
    subject_key BYTEA NOT NULL,
    window_started_at TIMESTAMPTZ NOT NULL,
    window_expires_at TIMESTAMPTZ NOT NULL,
    used BIGINT NOT NULL,
    PRIMARY KEY (config_fingerprint, subject_key),
    CONSTRAINT runlimit_fixed_windows_policy_id_size
        CHECK (octet_length(policy_id) BETWEEN 1 AND 128),
    CONSTRAINT runlimit_fixed_windows_scope_id_size
        CHECK (octet_length(scope_id) BETWEEN 1 AND 128),
    CONSTRAINT runlimit_fixed_windows_fingerprint_size
        CHECK (octet_length(config_fingerprint) = 32),
    CONSTRAINT runlimit_fixed_windows_subject_key_size
        CHECK (octet_length(subject_key) = 32),
    CONSTRAINT runlimit_fixed_windows_valid_window
        CHECK (window_expires_at > window_started_at),
    CONSTRAINT runlimit_fixed_windows_positive_usage
        CHECK (used > 0)
);

CREATE INDEX runlimit_fixed_windows_expiry_idx
    ON runlimit_fixed_windows (window_expires_at);
