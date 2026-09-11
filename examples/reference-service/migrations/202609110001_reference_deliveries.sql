CREATE TABLE reference_records (
    id uuid PRIMARY KEY,
    owner_id uuid NOT NULL,
    generation bigint NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (owner_id, id),
    CONSTRAINT chk_reference_records_generation_positive CHECK (generation > 0)
);

CREATE TABLE reference_delivery_commands (
    owner_id uuid NOT NULL,
    idempotency_key text NOT NULL,
    delivery_id uuid NOT NULL,
    record_id uuid NOT NULL,
    expected_generation bigint NOT NULL,
    request_payload jsonb NOT NULL,
    enqueue_job_type text NOT NULL,
    enqueue_idempotency_key text NOT NULL,
    enqueue_payload jsonb NOT NULL,
    enqueue_priority integer NOT NULL,
    enqueue_max_attempts integer NOT NULL,
    enqueue_timeout_seconds integer NOT NULL,
    enqueue_stage text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (owner_id, idempotency_key),
    UNIQUE (delivery_id),
    UNIQUE (owner_id, idempotency_key, delivery_id),
    CONSTRAINT fk_reference_delivery_commands_record
        FOREIGN KEY (owner_id, record_id)
        REFERENCES reference_records (owner_id, id) ON DELETE RESTRICT
        DEFERRABLE INITIALLY DEFERRED,
    CONSTRAINT chk_reference_delivery_commands_key
        CHECK (
            octet_length(idempotency_key) BETWEEN 1 AND 128
            AND idempotency_key ~ '^[A-Za-z0-9._:-]+$'
        ),
    CONSTRAINT chk_reference_delivery_commands_generation_positive
        CHECK (expected_generation > 0),
    CONSTRAINT chk_reference_delivery_commands_enqueue_key
        CHECK (octet_length(enqueue_idempotency_key) BETWEEN 1 AND 128),
    CONSTRAINT chk_reference_delivery_commands_attempts_positive
        CHECK (enqueue_max_attempts > 0),
    CONSTRAINT chk_reference_delivery_commands_timeout_positive
        CHECK (enqueue_timeout_seconds > 0),
    CONSTRAINT chk_reference_delivery_commands_stage_not_blank
        CHECK (length(btrim(enqueue_stage)) > 0)
);

CREATE TABLE reference_deliveries (
    id uuid PRIMARY KEY,
    owner_id uuid NOT NULL,
    idempotency_key text NOT NULL,
    job_id uuid NOT NULL UNIQUE,
    accepted_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT fk_reference_deliveries_command
        FOREIGN KEY (owner_id, idempotency_key, id)
        REFERENCES reference_delivery_commands (owner_id, idempotency_key, delivery_id)
        ON DELETE RESTRICT,
    CONSTRAINT fk_reference_deliveries_job
        FOREIGN KEY (job_id) REFERENCES job_queue (id) ON DELETE RESTRICT
);

CREATE INDEX idx_reference_deliveries_owner_id
    ON reference_deliveries (owner_id, id);
