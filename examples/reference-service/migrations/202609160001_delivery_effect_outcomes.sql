CREATE TABLE reference_delivery_effects (
    delivery_id uuid PRIMARY KEY,
    owner_id uuid NOT NULL,
    record_id uuid NOT NULL,
    record_generation bigint NOT NULL,
    provider_key text NOT NULL UNIQUE,
    provider_payload jsonb NOT NULL,
    state text NOT NULL DEFAULT 'AWAITING_ATTEMPT',
    provider_effect_id text,
    outcome_code text,
    dispatch_possible_at timestamptz,
    resolve_before timestamptz,
    acceptance_possible boolean NOT NULL DEFAULT false,
    updated_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT fk_reference_delivery_effects_delivery
        FOREIGN KEY (delivery_id) REFERENCES reference_deliveries (id) ON DELETE RESTRICT,
    CONSTRAINT fk_reference_delivery_effects_command
        FOREIGN KEY (owner_id, record_id)
        REFERENCES reference_records (owner_id, id) ON DELETE RESTRICT
        DEFERRABLE INITIALLY DEFERRED,
    CONSTRAINT chk_reference_delivery_effects_generation_positive
        CHECK (record_generation > 0),
    CONSTRAINT chk_reference_delivery_effects_key
        CHECK (
            octet_length(provider_key) BETWEEN 1 AND 128
            AND provider_key ~ '^[A-Za-z0-9._:-]+$'
        ),
    CONSTRAINT chk_reference_delivery_effects_state
        CHECK (state IN (
            'AWAITING_ATTEMPT',
            'RETRYABLE_UNDISPATCHED',
            'RECONCILE_NEEDED',
            'CONFIRMED',
            'BUSINESS_DENIED',
            'MANUAL_RESOLUTION',
            'EXHAUSTED'
        )),
    CONSTRAINT chk_reference_delivery_effects_provider_id
        CHECK (
            provider_effect_id IS NULL OR (
                octet_length(provider_effect_id) BETWEEN 1 AND 256
                AND provider_effect_id ~ '^[A-Za-z0-9._:-]+$'
            )
        ),
    CONSTRAINT chk_reference_delivery_effects_outcome_code
        CHECK (
            outcome_code IS NULL OR (
                octet_length(outcome_code) BETWEEN 1 AND 128
                AND outcome_code ~ '^[a-z0-9._-]+$'
            )
        ),
    CONSTRAINT chk_reference_delivery_effects_dispatch_window
        CHECK (
            (dispatch_possible_at IS NULL AND resolve_before IS NULL)
            OR (
                dispatch_possible_at IS NOT NULL
                AND resolve_before IS NOT NULL
                AND resolve_before > dispatch_possible_at
            )
        ),
    CONSTRAINT chk_reference_delivery_effects_confirmed_id
        CHECK (state <> 'CONFIRMED' OR provider_effect_id IS NOT NULL),
    CONSTRAINT chk_reference_delivery_effects_reconcile_uncertain
        CHECK (state <> 'RECONCILE_NEEDED' OR acceptance_possible)
);

INSERT INTO reference_delivery_effects (
    delivery_id,
    owner_id,
    record_id,
    record_generation,
    provider_key,
    provider_payload
)
SELECT
    c.delivery_id,
    c.owner_id,
    c.record_id,
    c.expected_generation,
    'reference-delivery:' || c.delivery_id::text,
    jsonb_build_object(
        'version', 1,
        'effect_id', c.delivery_id,
        'owner_id', c.owner_id,
        'record_id', c.record_id,
        'record_generation', c.expected_generation,
        'payload', c.request_payload
    )
FROM reference_delivery_commands c
JOIN reference_deliveries d ON d.id = c.delivery_id;

CREATE INDEX idx_reference_delivery_effects_owner_delivery
    ON reference_delivery_effects (owner_id, delivery_id);
