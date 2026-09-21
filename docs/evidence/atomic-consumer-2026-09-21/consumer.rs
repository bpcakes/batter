//! Compile-only downstream composition. Requires PostgreSQL 18, Runledger's
//! migrations and schema.sql. No retry, migration or database execution here.

use batter_sqlx::{PgAtomicError, PgAtomicUncertainty, PgScopeError, PgTransactionError};
use runledger_postgres::{
    DbPool, Error as RunledgerError,
    jobs::{
        JobEnqueue, JobEnqueueIntent, JobEnqueueIntentOutcome, JobEnqueueIntentOutcomeState,
        JobEnqueueOutcome,
    },
    run_atomic,
};
use sqlx::types::Uuid;

/// Original inputs survive failures; the caller also retains them if cancelled.
/// These contain sensitive payloads and keys and deliberately do not derive Debug.
pub struct Reconciliation<'a> {
    pub audit_id: Uuid,
    pub intent: JobEnqueueIntent<'a>,
    pub request: JobEnqueue<'a>,
}

/// Observations made before completion; neither identifier proves persistence.
pub struct ProvisionalWrites {
    pub audit_id: Uuid,
    pub intent: JobEnqueueIntentOutcome,
    pub enqueue: JobEnqueueOutcome,
}

/// Constructed only from the runner's acknowledged-commit branch.
pub struct Committed {
    writes: ProvisionalWrites,
}

impl Committed {
    pub fn job_id(&self) -> Uuid {
        self.writes.enqueue.job_id
    }

    /// Preserve inserted/existing disposition, protected job state, intent ID
    /// and observed intent lifecycle state. Intent state is a point-in-time read.
    pub fn receipt(&self) -> &ProvisionalWrites {
        &self.writes
    }
}

/// Body failure retains both stage and every prior provisional observation.
pub enum BodyFailure {
    Audit(PgScopeError<sqlx::Error>),
    Intent(PgScopeError<RunledgerError>),
    ConflictedIntent(JobEnqueueIntentOutcome),
    UnsupportedIntentState(JobEnqueueIntentOutcome),
    Enqueue {
        intent: JobEnqueueIntentOutcome,
        cause: PgScopeError<RunledgerError>,
    },
}

pub enum Disposition {
    /// Setup failed before any body operation ran.
    Begin(PgTransactionError),
    /// The runner acknowledged rollback of this attempt.
    RolledBack(BodyFailure),
    /// Exhaustively retains CommitUnconfirmed, RollbackUnconfirmed or ScopeLost,
    /// including original causes and provisional output/body rejection.
    Uncertain(PgAtomicUncertainty<ProvisionalWrites, BodyFailure>),
}

pub struct Failure<'a> {
    pub reconciliation: Reconciliation<'a>,
    pub disposition: Disposition,
}

/// Insert one audit row, record an intent, and enqueue in one owned transaction.
///
/// The caller supplies a stable audit_id and authorized Runledger inputs. The
/// intent and immediate request may describe separate work: no correspondence
/// is assumed. If they represent the same job, the caller must construct them
/// with identical scoped idempotency keys and canonical request fields.
///
/// An existing conflicted intent rejects this attempt. Pending and promoted
/// observations are retained, and unknown future intent states reject safely.
/// Recording an intent does not itself promote it or link the immediate job.
///
/// Only Ok(Committed) exposes an acknowledged job_id. IDs in error evidence are
/// provisional. This function never retries. Cancellation/panic returns no
/// disposition and proves no rollback; retain the original inputs and audit_id
/// outside this future for reconciliation. An absent row while the server may
/// still settle is not rollback evidence.
pub async fn audit_intent_and_enqueue<'a>(
    pool: &DbPool,
    audit_id: Uuid,
    intent: &JobEnqueueIntent<'a>,
    request: &JobEnqueue<'a>,
) -> Result<Committed, Failure<'a>> {
    let result = run_atomic(pool, async |mut scope| {
        scope
            .application(async |sql| {
                sqlx::query_scalar::<_, Uuid>(
                    "INSERT INTO public.application_audit (id, action) \
                     VALUES ($1, $2) RETURNING id",
                )
                .bind(audit_id)
                .bind("atomic_job_submission")
                .fetch_one(sql.executor())
                .await
            })
            .await
            .map_err(BodyFailure::Audit)?;

        let intent_outcome = scope
            .record_job_enqueue_intent(intent)
            .await
            .map_err(BodyFailure::Intent)?;
        match intent_outcome.state {
            JobEnqueueIntentOutcomeState::Pending
            | JobEnqueueIntentOutcomeState::Promoted { .. } => {}
            JobEnqueueIntentOutcomeState::Conflicted => {
                return Err(BodyFailure::ConflictedIntent(intent_outcome));
            }
            _ => return Err(BodyFailure::UnsupportedIntentState(intent_outcome)),
        }

        let mut queue = scope.queue();
        let enqueue = match queue.enqueue_job(request).await {
            Ok(outcome) => outcome,
            Err(cause) => {
                return Err(BodyFailure::Enqueue {
                    intent: intent_outcome,
                    cause,
                });
            }
        };
        Ok(ProvisionalWrites {
            audit_id,
            intent: intent_outcome,
            enqueue,
        })
    })
    .await;

    let disposition = match result {
        Ok(writes) => return Ok(Committed { writes }),
        Err(PgAtomicError::Begin(cause)) => Disposition::Begin(cause),
        Err(PgAtomicError::Rejected(rejection)) => Disposition::RolledBack(rejection),
        Err(PgAtomicError::Uncertain(uncertainty)) => Disposition::Uncertain(uncertainty),
    };
    Err(Failure {
        reconciliation: Reconciliation {
            audit_id,
            intent: intent.clone(),
            request: request.clone(),
        },
        disposition,
    })
}
