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
    /// writes, success, handler continuation to a further run, retry, terminal
    /// failure, dead-letter writes, execution resource claims with their expiry
    /// reaping, lease reaping, and the workflow-step hooks every claim,
    /// continuation and settlement runs.
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
