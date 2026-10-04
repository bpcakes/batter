//! The supported operation-to-object inventory for this source version.
//!
//! Each entry names the exact relation, relation-level privileges PostgreSQL
//! cannot express per column, and the column lists the statements of this
//! version read, insert and update. Entries are sorted by relation and column so
//! a review diff stays readable, and a native query or migration change is
//! expected to change this file.

pub(super) const INTENTS: &str = "job_enqueue_intents";
pub(super) const QUEUE: &str = "job_queue";
pub(super) const ATTEMPTS: &str = "job_attempts";
pub(super) const EVENTS: &str = "job_events";
pub(super) const DEAD_LETTERS: &str = "job_dead_letters";
pub(super) const RESOURCE_CLAIMS: &str = "job_execution_resource_claims";
pub(super) const DEFINITIONS: &str = "job_definitions";
pub(super) const SCHEDULES: &str = "job_schedules";
pub(super) const WORKFLOW_STEPS: &str = "workflow_steps";
pub(super) const WORKFLOW_ACTIVE_CLAIMS: &str = "workflow_active_claims";
pub(super) const WORKFLOW_RUNS: &str = "workflow_runs";
pub(super) const SQLX_HISTORY: &str = "_sqlx_migrations";
pub(super) const RUNLEDGER_HISTORY: &str = "runledger_migration_history";

mod tables;

use batter_sqlx::verification::ObjectPrivilege;
use tables::{
    CATALOG_SYNC, DIRECT_JOB_EXECUTION, INTENT_PROMOTION, INTENT_SUBMISSION, SCHEDULED_DISPATCH,
    SCHEMA_SNAPSHOT,
};

/// One independently selectable native Runledger operation group.
///
/// Selections are operations, never application role names, and there is no
/// administrator preset. Arbitrary workflow execution, replay and recovery
/// administration, arbitrary reader APIs, requeue and history pruning are
/// outside the supported contract and have no selection here.
///
/// Each variant names the entrypoints of this source version it covers, and the
/// neighbouring ones it does not. A covered entrypoint is one whose statements
/// this version's requirements were derived from; an entrypoint absent from
/// every list has no selection, so its privileges are not published here and
/// granting a group does not make it supported. These are the entrypoints, not
/// the privileges: the exact relations, columns and privileges each group
/// requires are the `Requirement` tables in this module's `tables` submodule.
///
/// Excluded from every group, because they are administrative or arbitrary
/// readers rather than serving operations: `update_job_definition`,
/// `cancel_job`, `compare_and_requeue_job`, `compare_and_replay_succeeded_job`,
/// `delete_promoted_job_enqueue_intents_before`, `insert_job_log`, the
/// `list_*`, `get_*` and `*_metrics` readers, and the runtime-config and
/// workflow-execution APIs.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum RunledgerOperation {
    /// Protected required-intent recording and its duplicate/conflict
    /// resolution, in global and organization scopes, inside an application
    /// transaction. This reads selected identity, status and request columns and
    /// takes the row lock its duplicate resolution needs; it carries no queue
    /// authority and cannot promote an intent.
    ///
    /// Covers `record_job_enqueue_intent` and `record_job_enqueue_intent_tx`.
    /// Excludes every intent reader and
    /// `delete_promoted_job_enqueue_intents_before`.
    IntentSubmission,
    /// The privileged read-only schema compatibility snapshot. This locks and
    /// reads whole relations, so it is always a separate explicit selection.
    ///
    /// Covers `ensure_schema_compatible_after_idempotency_cutover`, which is
    /// what `batter::runledger::verify_schema` calls. Excludes `migrate` and the
    /// pre-cutover `ensure_schema_compatible`: applying migrations is an owner
    /// operation, not a serving one.
    SchemaSnapshot,
    /// Direct-job worker execution: claiming, release of an unstarted claim
    /// after failed `RUNNING` persistence, heartbeat, progress and checkpoint
    /// writes, success, handler continuation to a further run, retry, terminal
    /// failure, dead-letter writes, execution resource claims with their expiry
    /// reaping, lease reaping, and the workflow-step hooks every claim,
    /// continuation and settlement runs.
    ///
    /// This supports a direct-only workload. Workflow-linked rows reach the same
    /// hooks and reaper, so it establishes neither workflow isolation nor
    /// mixed-workload support.
    ///
    /// Covers `claim_jobs`, `claim_jobs_for_types`, `claim_prestart_jobs`,
    /// `claim_prestart_jobs_for_types`, `mark_job_running`,
    /// `release_unstarted_job_claim`, `heartbeat_job`,
    /// `update_job_ordinary_progress`, `complete_job_success`,
    /// `complete_job_failure`, `complete_job_continuation` and
    /// `reap_expired_leases_with_diagnostics`, each with the `_for_lease` and
    /// `_with_outcome` forms the native worker calls.
    ///
    /// Excludes every enqueue entrypoint. This group holds no `INSERT` on
    /// `job_queue` at all, so it cannot submit work: the promoted enqueue
    /// belongs to [`Self::IntentPromotion`] and the materialized scheduled job
    /// to [`Self::ScheduledDispatch`]. Also excludes
    /// `reap_expired_leases_with_terminal_records` and the deprecated
    /// stage-bearing `update_job_progress`, neither of which this version's
    /// requirements were derived from.
    DirectJobExecution,
    /// Durable intent promotion, including its enqueue of the promoted job.
    ///
    /// Covers `promote_job_enqueue_intents_for_types`. Excludes
    /// `delete_promoted_job_enqueue_intents_before` and
    /// `delete_promoted_job_enqueue_intents_for_jobs_tx`: pruning promoted
    /// intents is history administration, not promotion.
    IntentPromotion,
    /// Job-definition and schedule catalog synchronization, including the exact
    /// disable and deactivation of entries absent from the catalog.
    ///
    /// Covers `upsert_job_definition_tx`,
    /// `sync_catalog_job_definitions_exact_tx`, `upsert_job_schedule` and its
    /// `_tx` form, `sync_catalog_job_schedules_tx`,
    /// `deactivate_schedules_absent_from_names_tx` and
    /// `prepare_schedule_exact_sync_critical_section_tx`.
    ///
    /// Excludes `update_job_definition`, whose `RETURNING` list is wider than
    /// synchronization needs and which is an administrative API, and the
    /// single-schedule `set_job_schedule_active` and
    /// `set_job_schedule_next_fire_at` operators.
    CatalogSync,
    /// Scheduled dispatch of due direct jobs, including recording the fire and
    /// deferring a schedule whose cron expression cannot be advanced.
    ///
    /// Covers `claim_due_schedules_tx`, `mark_schedule_fired_tx` and the
    /// `enqueue_job_tx` the dispatcher performs for the job it materializes,
    /// which is why this group carries `INSERT` on `job_queue`. Excludes
    /// `get_job_schedule_by_name` and the catalog-synchronization entrypoints,
    /// which are [`Self::CatalogSync`].
    ScheduledDispatch,
}

pub(super) struct Requirement {
    pub(super) relation: &'static str,
    pub(super) relation_privileges: &'static [ObjectPrivilege],
    pub(super) select: &'static [&'static str],
    pub(super) insert: &'static [&'static str],
    pub(super) update: &'static [&'static str],
}

pub(super) const fn read_only(relation: &'static str) -> Requirement {
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
