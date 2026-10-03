//! The supported operation-to-object inventory for this source version.
//!
//! Each entry names the exact relation, relation-level privileges PostgreSQL
//! cannot express per column, and the column lists the statements of this
//! version read, insert and update. Entries are sorted by relation and column so
//! a review diff stays readable, and a native query or migration change is
//! expected to change this file.

const INTENTS: &str = "job_enqueue_intents";
const QUEUE: &str = "job_queue";
const ATTEMPTS: &str = "job_attempts";
const EVENTS: &str = "job_events";
const DEAD_LETTERS: &str = "job_dead_letters";
const RESOURCE_CLAIMS: &str = "job_execution_resource_claims";
const DEFINITIONS: &str = "job_definitions";
const SCHEDULES: &str = "job_schedules";
const WORKFLOW_STEPS: &str = "workflow_steps";
const WORKFLOW_RUNS: &str = "workflow_runs";
const SQLX_HISTORY: &str = "_sqlx_migrations";
const RUNLEDGER_HISTORY: &str = "runledger_migration_history";

use batter_sqlx::verification::ObjectPrivilege;

/// One independently selectable native Runledger operation group.
///
/// Selections are operations, never application role names, and there is no
/// administrator preset. Arbitrary workflow execution, replay and recovery
/// administration, arbitrary reader APIs, requeue and history pruning are
/// outside the supported contract and have no selection here.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum RunledgerOperation {
    /// Protected required-intent recording and its duplicate/conflict
    /// resolution, in global and organization scopes, inside an application
    /// transaction. This reads selected identity, status and request columns and
    /// takes the row lock its duplicate resolution needs; it carries no queue
    /// authority and cannot promote an intent.
    IntentSubmission,
    /// The privileged read-only schema compatibility snapshot. This locks and
    /// reads whole relations, so it is always a separate explicit selection.
    SchemaSnapshot,
    /// Direct-job worker execution: claiming, release of an unstarted claim
    /// after failed `RUNNING` persistence, heartbeat, progress and checkpoint
    /// writes, success, retry, terminal failure, dead-letter writes, execution
    /// resource claims with their expiry reaping, lease reaping, and the
    /// workflow-step hooks every claim and settlement runs.
    ///
    /// This supports a direct-only workload. Workflow-linked rows reach the same
    /// hooks and reaper, so it establishes neither workflow isolation nor
    /// mixed-workload support.
    DirectJobExecution,
    /// Durable intent promotion, including its enqueue of the promoted job.
    IntentPromotion,
    /// Job-definition and schedule catalog synchronization, including the exact
    /// disable and deactivation of entries absent from the catalog.
    CatalogSync,
    /// Scheduled dispatch of due direct jobs, including recording the fire and
    /// deferring a schedule whose cron expression cannot be advanced.
    ScheduledDispatch,
}

pub(super) struct Requirement {
    pub(super) relation: &'static str,
    pub(super) relation_privileges: &'static [ObjectPrivilege],
    pub(super) select: &'static [&'static str],
    pub(super) insert: &'static [&'static str],
    pub(super) update: &'static [&'static str],
}

const fn read_only(relation: &'static str) -> Requirement {
    Requirement {
        relation,
        relation_privileges: &[ObjectPrivilege::Select],
        select: &[],
        insert: &[],
        update: &[],
    }
}

pub(super) const fn requirements(operation: RunledgerOperation) -> &'static [Requirement] {
    match operation {
        RunledgerOperation::IntentSubmission => INTENT_SUBMISSION,
        RunledgerOperation::SchemaSnapshot => SCHEMA_SNAPSHOT,
        RunledgerOperation::DirectJobExecution => DIRECT_JOB_EXECUTION,
        RunledgerOperation::IntentPromotion => INTENT_PROMOTION,
        RunledgerOperation::CatalogSync => CATALOG_SYNC,
        RunledgerOperation::ScheduledDispatch => SCHEDULED_DISPATCH,
    }
}

const INTENT_SUBMISSION: &[Requirement] = &[Requirement {
    relation: INTENTS,
    relation_privileges: &[],
    select: &[
        "enqueue_request",
        "id",
        "idempotency_key",
        "job_type",
        "organization_id",
        "promoted_job_id",
        "status",
    ],
    insert: &[
        "enqueue_request",
        "enqueue_request_version",
        "execution_resource_key",
        "idempotency_key",
        "job_type",
        "max_attempts",
        "next_run_at",
        "organization_id",
        "payload",
        "priority",
        "stage",
        "timeout_seconds",
    ],
    update: &["id"],
}];

const SCHEMA_SNAPSHOT: &[Requirement] = &[
    read_only(QUEUE),
    read_only(RUNLEDGER_HISTORY),
    read_only(SQLX_HISTORY),
    read_only(WORKFLOW_RUNS),
    read_only(WORKFLOW_STEPS),
];

const DIRECT_JOB_EXECUTION: &[Requirement] = &[
    Requirement {
        relation: ATTEMPTS,
        relation_privileges: &[ObjectPrivilege::Delete],
        select: &[
            "attempt",
            "claim_origin",
            "execution_started_persisted_at",
            "job_id",
            "run_number",
        ],
        insert: &[
            "attempt",
            "claim_origin",
            "execution_started_persisted_at",
            "job_id",
            "leased_at",
            "run_number",
            "started_at",
            "worker_id",
        ],
        update: &[
            "effective_next_run_at",
            "error_code",
            "error_message",
            "execution_started_persisted_at",
            "finished_at",
            "outcome",
            "requested_retry_not_before",
            "retry_delay_ms",
            "retry_timing_source",
        ],
    },
    Requirement {
        relation: DEAD_LETTERS,
        relation_privileges: &[],
        // The dead-letter upsert arbitrates on `job_id` and reads every column
        // it refreshes through `EXCLUDED`, which PostgreSQL treats as reading
        // the target relation's columns.
        select: &[
            "attempt",
            "checkpoint_snapshot",
            "error_code",
            "error_message",
            "failed_at",
            "job_id",
            "payload_snapshot",
            "run_number",
        ],
        insert: &[
            "attempt",
            "checkpoint_snapshot",
            "error_code",
            "error_message",
            "failed_at",
            "job_id",
            "job_type",
            "organization_id",
            "payload_snapshot",
            "run_number",
        ],
        update: &[
            "attempt",
            "checkpoint_snapshot",
            "error_code",
            "error_message",
            "failed_at",
            "payload_snapshot",
            "run_number",
        ],
    },
    Requirement {
        relation: EVENTS,
        relation_privileges: &[],
        select: &[
            "attempt",
            "event_type",
            "id",
            "job_id",
            "occurred_at",
            "run_number",
            "stage",
        ],
        insert: &[
            "attempt",
            "event_type",
            "job_id",
            "payload",
            "progress_done",
            "progress_total",
            "run_number",
            "stage",
        ],
        update: &[],
    },
    Requirement {
        relation: RESOURCE_CLAIMS,
        relation_privileges: &[ObjectPrivilege::Delete],
        select: &[
            "attempt",
            "job_id",
            "lease_expires_at",
            "release_after",
            "resource_key",
            "run_number",
            "worker_id",
        ],
        insert: &[
            "attempt",
            "job_id",
            "lease_expires_at",
            "resource_key",
            "run_number",
            "worker_id",
        ],
        update: &["lease_expires_at", "release_after"],
    },
    Requirement {
        relation: QUEUE,
        relation_privileges: &[],
        select: &[
            "attempt",
            "checkpoint",
            "created_at",
            "execution_resource_key",
            "finished_at",
            "id",
            "idempotency_key",
            "job_type",
            "last_error_code",
            "last_error_message",
            "last_heartbeat_at",
            "lease_expires_at",
            "max_attempts",
            "next_run_at",
            "organization_id",
            "output",
            "payload",
            "priority",
            "progress_done",
            "progress_pct",
            "progress_total",
            "run_number",
            "stage",
            "started_at",
            "status",
            "status_reason",
            "timeout_seconds",
            "updated_at",
            "worker_id",
        ],
        insert: &[],
        update: &[
            "attempt",
            "checkpoint",
            "finished_at",
            "last_error_code",
            "last_error_message",
            "last_heartbeat_at",
            "lease_expires_at",
            "next_run_at",
            "output",
            "progress_done",
            "progress_total",
            "stage",
            "started_at",
            "status",
            "status_reason",
            "updated_at",
            "worker_id",
        ],
    },
    Requirement {
        relation: WORKFLOW_STEPS,
        relation_privileges: &[],
        // `started_at` is read by the claim and claim-release hooks' own SET
        // expressions, so it needs SELECT alongside UPDATE.
        select: &["job_id", "started_at", "status"],
        insert: &[],
        update: &[
            "finished_at",
            "last_error_code",
            "last_error_message",
            "output",
            "started_at",
            "status",
            "status_reason",
            "updated_at",
        ],
    },
];

const INTENT_PROMOTION: &[Requirement] = &[
    Requirement {
        relation: DEFINITIONS,
        relation_privileges: &[],
        select: &[
            "default_priority",
            "default_timeout_seconds",
            "is_enabled",
            "job_type",
            "max_attempts",
        ],
        insert: &[],
        update: &["updated_at"],
    },
    Requirement {
        relation: EVENTS,
        relation_privileges: &[],
        select: &[],
        insert: &["event_type", "job_id", "payload", "run_number", "stage"],
        update: &[],
    },
    Requirement {
        relation: INTENTS,
        relation_privileges: &[],
        select: &[
            "created_at",
            "enqueue_request",
            "enqueue_request_version",
            "execution_resource_key",
            "id",
            "idempotency_key",
            "job_type",
            "max_attempts",
            "next_promotion_at",
            "next_run_at",
            "organization_id",
            "payload",
            "priority",
            "promotion_attempts",
            "stage",
            "status",
            "timeout_seconds",
        ],
        insert: &[],
        update: &[
            "conflicted_at",
            "last_attempted_at",
            "last_error_code",
            "last_error_message",
            "next_promotion_at",
            "promoted_at",
            "promoted_job_id",
            "promotion_attempts",
            "status",
        ],
    },
    Requirement {
        relation: QUEUE,
        relation_privileges: &[],
        select: &[
            "enqueue_request",
            "id",
            "idempotency_key",
            "job_type",
            "organization_id",
            "run_number",
            "status",
        ],
        insert: &[
            "enqueue_request",
            "execution_resource_key",
            "idempotency_key",
            "job_type",
            "max_attempts",
            "next_run_at",
            "organization_id",
            "payload",
            "priority",
            "stage",
            "timeout_seconds",
        ],
        update: &["id"],
    },
];

const CATALOG_SYNC: &[Requirement] = &[
    Requirement {
        relation: DEFINITIONS,
        relation_privileges: &[ObjectPrivilege::Maintain],
        select: &[
            "created_at",
            "default_priority",
            "default_timeout_seconds",
            "is_enabled",
            "job_type",
            "max_attempts",
            "updated_at",
            "version",
        ],
        insert: &[
            "default_priority",
            "default_timeout_seconds",
            "is_enabled",
            "job_type",
            "max_attempts",
            "version",
        ],
        update: &[
            "default_priority",
            "default_timeout_seconds",
            "is_enabled",
            "max_attempts",
            "updated_at",
            "version",
        ],
    },
    Requirement {
        relation: SCHEDULES,
        relation_privileges: &[ObjectPrivilege::Maintain],
        select: &[
            "cron_expr",
            "id",
            "is_active",
            "job_type",
            "max_jitter_seconds",
            "name",
            "next_fire_at",
            "organization_id",
            "payload_template",
            "timezone",
        ],
        insert: &[
            "cron_expr",
            "is_active",
            "job_type",
            "max_jitter_seconds",
            "name",
            "next_fire_at",
            "organization_id",
            "payload_template",
            "timezone",
        ],
        update: &[
            "cron_expr",
            "is_active",
            "job_type",
            "max_jitter_seconds",
            "next_fire_at",
            "payload_template",
            "timezone",
            "updated_at",
        ],
    },
];

const SCHEDULED_DISPATCH: &[Requirement] = &[
    Requirement {
        relation: DEFINITIONS,
        relation_privileges: &[],
        select: &[
            "default_priority",
            "default_timeout_seconds",
            "is_enabled",
            "job_type",
            "max_attempts",
        ],
        insert: &[],
        update: &["updated_at"],
    },
    Requirement {
        relation: EVENTS,
        relation_privileges: &[],
        select: &[],
        insert: &["event_type", "job_id", "payload", "run_number", "stage"],
        update: &[],
    },
    Requirement {
        relation: QUEUE,
        relation_privileges: &[],
        select: &["id", "run_number", "status"],
        insert: &[
            "enqueue_request",
            "execution_resource_key",
            "idempotency_key",
            "job_type",
            "max_attempts",
            "next_run_at",
            "organization_id",
            "payload",
            "priority",
            "stage",
            "timeout_seconds",
        ],
        update: &[],
    },
    Requirement {
        relation: SCHEDULES,
        relation_privileges: &[ObjectPrivilege::Maintain],
        select: &[
            "cron_expr",
            "id",
            "is_active",
            "job_type",
            "max_jitter_seconds",
            "name",
            "next_fire_at",
            "organization_id",
            "payload_template",
        ],
        insert: &[],
        update: &["last_fired_at", "next_fire_at", "updated_at"],
    },
];
