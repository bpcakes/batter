//! One snapshot of retained intent lineage, without payload or diagnostic access.

use crate::jobs::JobScope;
use crate::{Error, PgTransactionalExecutor, QueryError, QueryErrorCategory, Result};
use runledger_core::jobs::{JobStatus, JobTypeName};
use sqlx::types::Uuid;

/// Point-in-time evidence, not permission to submit or replay work.
///
/// `NotFound` also covers a different scope and expired retention. It does not
/// establish that an earlier transaction rolled back. Reconcile business state
/// and preserve the original idempotency request before deciding to retry.
#[derive(Clone, Debug)]
pub enum IntentReconciliation {
    /// No retained intent was visible for this ID in the exact requested scope.
    NotFound,
    /// Intent, promoted job and current definition read in one SQL snapshot.
    Retained(RetainedIntent),
}

/// Library-validated correlation with no payload, checkpoint or diagnostic fields.
/// The observation may change as soon as the query finishes.
#[derive(Clone, Debug)]
pub struct RetainedIntent {
    id: Uuid,
    job_type: JobTypeName,
    scope: JobScope,
    state: IntentCorrelationState,
    definition: Option<CorrelatedDefinition>,
}

impl RetainedIntent {
    /// Observed database identity.
    pub fn id(&self) -> Uuid {
        self.id
    }
    /// Validated job type shared by the intent and any promoted job.
    pub fn job_type(&self) -> &JobTypeName {
        &self.job_type
    }
    /// Exact scope selected and observed by the reader.
    pub fn scope(&self) -> JobScope {
        self.scope
    }
    /// Retained promotion state and its phase-specific correlation.
    pub fn state(&self) -> &IntentCorrelationState {
        &self.state
    }
    /// Current definition, not a claim about the definition used at promotion.
    pub fn definition(&self) -> Option<&CorrelatedDefinition> {
        self.definition.as_ref()
    }
}

/// A promoted observation always includes a job of the same type and scope.
#[derive(Clone, Debug)]
pub enum IntentCorrelationState {
    /// Awaiting promotion, including deferred attempts.
    Pending,
    /// Promotion retained a conflict; this is not an accepted handoff.
    Conflicted,
    /// The retained intent is linked to this matching job.
    Promoted(CorrelatedJob),
}

/// Selected metadata of the promoted job in the same snapshot as the intent.
#[derive(Clone, Debug)]
pub struct CorrelatedJob {
    id: Uuid,
    status: JobStatus,
    run_number: i32,
}
impl CorrelatedJob {
    /// Observed database identity.
    pub fn id(&self) -> Uuid {
        self.id
    }
    /// Job status observed in the same snapshot as the intent.
    pub fn status(&self) -> JobStatus {
        self.status
    }
    /// Positive run identity at observation time.
    pub fn run_number(&self) -> i32 {
        self.run_number
    }
}

/// Current definition metadata; no configuration or payload is exposed.
#[derive(Clone, Debug)]
pub struct CorrelatedDefinition {
    version: i32,
    enabled: bool,
}
impl CorrelatedDefinition {
    /// Current application-declared definition version.
    pub fn version(&self) -> i32 {
        self.version
    }
    /// Whether the current definition accepts dispatch.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }
}

#[derive(sqlx::FromRow)]
struct Row {
    id: Uuid,
    job_type: String,
    intent_status: String,
    promoted_job_id: Option<Uuid>,
    job_id: Option<Uuid>,
    job_status: Option<String>,
    run_number: Option<i32>,
    definition_version: Option<i32>,
    definition_enabled: Option<bool>,
}

pub(super) async fn read(
    tx: &mut impl PgTransactionalExecutor,
    scope: JobScope,
    intent_id: Uuid,
) -> Result<IntentReconciliation> {
    // No row locks: ordinary snapshot reads neither invert intent/queue lock
    // ordering nor wait for promotion. The joins must not expose another scope,
    // even if privileged out-of-band SQL created an invalid lineage link.
    let row = sqlx::query_as::<_, Row>(
        "SELECT i.id, i.job_type, i.status AS intent_status, i.promoted_job_id,
                q.id AS job_id, q.status::text AS job_status, q.run_number,
                d.version AS definition_version, d.is_enabled AS definition_enabled
         FROM job_enqueue_intents i
         LEFT JOIN job_queue q ON q.id = i.promoted_job_id
              AND q.organization_id IS NOT DISTINCT FROM i.organization_id
              AND q.job_type = i.job_type
         LEFT JOIN job_definitions d ON d.job_type = i.job_type
         WHERE i.id = $1 AND i.organization_id IS NOT DISTINCT FROM $2",
    )
    .bind(intent_id)
    .bind(scope.organization_id())
    .fetch_optional(tx.executor())
    .await
    .map_err(|error| Error::from_query_sqlx_with_context("reconcile enqueue intent", error))?;
    let Some(row) = row else {
        return Ok(IntentReconciliation::NotFound);
    };
    let state = match (row.intent_status.as_str(), row.promoted_job_id, row.job_id) {
        ("PENDING", None, None) => IntentCorrelationState::Pending,
        ("CONFLICTED", None, None) => IntentCorrelationState::Conflicted,
        ("PROMOTED", Some(expected), Some(id)) if expected == id => {
            let status = row
                .job_status
                .ok_or_else(invalid)?
                .parse()
                .map_err(|_| invalid())?;
            let run_number = row.run_number.filter(|n| *n > 0).ok_or_else(invalid)?;
            IntentCorrelationState::Promoted(CorrelatedJob {
                id,
                status,
                run_number,
            })
        }
        _ => return Err(invalid()),
    };
    let definition = match (row.definition_version, row.definition_enabled) {
        (None, None) => None,
        (Some(version), Some(enabled)) => Some(CorrelatedDefinition { version, enabled }),
        _ => return Err(invalid()),
    };
    let job_type = JobTypeName::try_from(row.job_type).map_err(|_| invalid())?;
    Ok(IntentReconciliation::Retained(RetainedIntent {
        id: row.id,
        job_type,
        scope,
        state,
        definition,
    }))
}

fn invalid() -> Error {
    Error::QueryError(QueryError::from_classified(
        QueryErrorCategory::Internal,
        "job.invalid_intent_correlation",
        "Retained enqueue intent correlation is invalid.",
        "intent correlation requires a valid state and a same-scope, same-type promoted job",
    ))
}
