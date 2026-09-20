use super::*;
mod progress;

use batter::operation::{OperationContext, OperationError};
use batter::{
    runledger::RunledgerTransaction,
    sqlx::{PgLease, PgScopeError, PgScopedSql, PgSession, PgTransactionError, SqlxFailure},
};
use runledger_postgres::jobs::{JobEnqueue, JobEnqueueDisposition, JobEnqueueOutcome};
use sqlx::PgPool;

use progress::{SettledSubmission, SubmissionProgress};

impl DeliveryService {
    /// Bind delivery commands to the application-owned native pool.
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Submit one exact, owner-scoped delivery command under the supplied total budget.
    ///
    /// Validation precedes acquisition. Once `BEGIN` is acknowledged, the command
    /// identity, delivery and Runledger enqueue use that single transaction. No
    /// failure is retried automatically.
    pub async fn submit(
        &self,
        context: &OperationContext,
        owner: OwnerId,
        record_id: Uuid,
        request: SubmitDelivery,
    ) -> Result<SubmitResult, SubmitError> {
        let request = request.validate(record_id).map_err(SubmitError::Invalid)?;
        let progress = SubmissionProgress::new();
        let progress_in_operation = progress.clone();
        let pool = self.pool.clone();
        let result = context
            .run_resolved(
                "delivery.submit",
                move |scope| async move {
                    attempt_submit(
                        &pool,
                        &scope,
                        &progress_in_operation,
                        owner,
                        record_id,
                        &request,
                    )
                    .await
                },
                move |result| progress.resolve(result),
            )
            .await;
        match result {
            Ok(result) => Ok(result),
            Err(OperationError::Interrupted(interruption)) => {
                Err(SubmitError::Interrupted(interruption))
            }
            Err(OperationError::Failed(error)) => Err(error),
        }
    }

    /// Query a delivery by its response-generated identity within the same owner scope.
    pub async fn get_by_id(
        &self,
        context: &OperationContext,
        owner: OwnerId,
        delivery_id: Uuid,
    ) -> Result<Option<Delivery>, QueryError> {
        if delivery_id.is_nil() {
            return Err(QueryError::Invalid(ValidationError::NilDelivery));
        }
        self.query(context, move |connection| {
            Box::pin(load_delivery_by_id(connection, owner, delivery_id))
        })
        .await
    }

    /// Reconcile by the owner-retained key without needing a POST response.
    pub async fn get_by_key(
        &self,
        context: &OperationContext,
        owner: OwnerId,
        idempotency_key: &str,
    ) -> Result<Option<Delivery>, QueryError> {
        validate_idempotency_key(idempotency_key).map_err(QueryError::Invalid)?;
        let key = idempotency_key.to_owned();
        self.query(context, move |connection| {
            Box::pin(async move {
                load_command(connection, owner, &key)
                    .await?
                    .map(CommandRow::delivery)
                    .transpose()
            })
        })
        .await
    }

    async fn query<F>(
        &self,
        context: &OperationContext,
        query: F,
    ) -> Result<Option<Delivery>, QueryError>
    where
        F: for<'a, 'connection> FnOnce(
                &'a mut PgSession<'connection>,
            ) -> std::pin::Pin<
                Box<
                    dyn std::future::Future<Output = Result<Option<Delivery>, StorageError>>
                        + Send
                        + 'a,
                >,
            > + Send
            + 'static,
    {
        let pool = self.pool.clone();
        let result = context
            .run("delivery.query", move |scope| {
                // Erase this private phase's layout while retaining acquisition
                // and lease destruction inside the operation boundary.
                let work: std::pin::Pin<
                    Box<
                        dyn std::future::Future<
                                Output = Result<Option<Delivery>, QueryAttemptError>,
                            > + Send,
                    >,
                > = Box::pin(async move {
                    let lease = PgLease::acquire(&pool, &scope)
                        .await
                        .map_err(QueryAttemptError::Acquire)?;
                    lease
                        .with_connection(async move |connection| {
                            query(connection).await.map_err(QueryAttemptError::Storage)
                        })
                        .await
                });
                work
            })
            .await;
        match result {
            Ok(value) => Ok(value),
            Err(OperationError::Interrupted(interruption)) => {
                Err(QueryError::Interrupted(interruption))
            }
            Err(OperationError::Failed(QueryAttemptError::Acquire(
                OperationError::Interrupted(interruption),
            ))) => Err(QueryError::Interrupted(interruption)),
            Err(OperationError::Failed(QueryAttemptError::Acquire(OperationError::Failed(
                error,
            )))) => Err(QueryError::Storage(StorageError::Sqlx(error.into_native()))),
            Err(OperationError::Failed(QueryAttemptError::Storage(error))) => {
                Err(QueryError::Storage(error))
            }
        }
    }
}

#[derive(Debug)]
enum QueryAttemptError {
    Acquire(OperationError<SqlxFailure>),
    Storage(StorageError),
}

#[derive(Debug)]
enum SubmitAttemptError {
    Begin(PgTransactionError),
    Commit(PgTransactionError),
    Scope(Box<PgScopeError<CommandFailure>>),
    Enqueue(Box<PgScopeError<runledger_postgres::Error>>),
    Rollback {
        operation: Box<CommandFailure>,
        rollback: PgTransactionError,
    },
}
impl SubmitAttemptError {
    fn into_submit_error(self) -> SubmitError {
        match self {
            Self::Begin(error) => SubmitError::Storage(StorageError::Transaction(error)),
            Self::Commit(error) => SubmitError::Uncertain(UncertainSubmission::Commit(error)),
            Self::Scope(error) => SubmitError::Uncertain(UncertainSubmission::Scope(error)),
            Self::Enqueue(error) => SubmitError::Uncertain(UncertainSubmission::Enqueue(error)),
            Self::Rollback {
                operation,
                rollback,
            } => SubmitError::Uncertain(UncertainSubmission::Rollback {
                operation,
                rollback,
            }),
        }
    }
}

async fn attempt_submit(
    pool: &PgPool,
    _context: &OperationContext,
    progress: &SubmissionProgress,
    owner: OwnerId,
    record_id: Uuid,
    request: &ValidatedSubmit,
) -> Result<SettledSubmission, SubmitAttemptError> {
    let tx = RunledgerTransaction::begin(pool)
        .await
        .map_err(SubmitAttemptError::Begin)?;
    let active = progress.transaction_began();
    let (tx, prepared) = tx
        .operation(async |sql| prepare_submission(sql, owner, record_id, request).await)
        .await
        .map_err(|error| SubmitAttemptError::Scope(Box::new(error)))?;
    let (tx, result) = match prepared {
        Err(error) => (tx, Err(error)),
        Ok(PreparedSubmission::Replayed(result)) => (tx, Ok(result)),
        Ok(PreparedSubmission::New(new, generation)) => {
            let enqueue = JobEnqueue {
                job_type: JobType::new(DELIVERY_JOB_TYPE),
                organization_id: Some(owner.as_uuid()),
                payload: &new.enqueue_payload,
                priority: Some(ENQUEUE_PRIORITY),
                max_attempts: Some(ENQUEUE_MAX_ATTEMPTS),
                timeout_seconds: Some(ENQUEUE_TIMEOUT_SECONDS),
                next_run_at: None,
                idempotency_key: Some(&new.enqueue_idempotency_key),
                stage: Some(JobStage::Queued),
            };
            let (tx, outcome) = tx
                .enqueue_job(&enqueue)
                .await
                .map_err(|error| SubmitAttemptError::Enqueue(Box::new(error)))?;
            tx.operation(async |sql| {
                if outcome.disposition != JobEnqueueDisposition::Inserted {
                    return Err(StorageError::Invariant(
                        "new command resolved to an existing upstream job",
                    )
                    .into());
                }
                insert_delivery(
                    sql,
                    owner,
                    request,
                    new.delivery_id,
                    &outcome,
                    &new.provider_key,
                    &new.provider_payload,
                )
                .await?;
                Ok(SubmitResult {
                    disposition: SubmitDisposition::Accepted,
                    delivery: Delivery {
                        delivery_id: new.delivery_id,
                        record_id,
                        generation,
                        payload: request.payload.clone(),
                        state: DeliveryState::from_job_status(outcome.status),
                        provider: ProviderOutcome {
                            state: ProviderEffectState::AwaitingAttempt,
                            provider_effect_id: None,
                            acceptance_possible: false,
                        },
                    },
                })
            })
            .await
            .map_err(|error| SubmitAttemptError::Scope(Box::new(error)))?
        }
    };
    match result {
        Ok(result) => {
            let _confirmed = tx
                .commit()
                .await
                .map_err(|error| SubmitAttemptError::Commit(error.into_cause()))?;
            Ok(active.committed(result))
        }
        Err(operation) => match tx.rollback().await {
            Ok(_confirmed) => Ok(active.rolled_back(operation)),
            Err(rollback) => Err(SubmitAttemptError::Rollback {
                operation: Box::new(operation),
                rollback,
            }),
        },
    }
}

enum PreparedSubmission {
    Replayed(SubmitResult),
    New(Box<NewDelivery>, i64),
}

struct NewDelivery {
    delivery_id: Uuid,
    enqueue_idempotency_key: String,
    enqueue_payload: Value,
    provider_key: String,
    provider_payload: ProviderEffectRequest,
}

fn prepare_new_delivery(
    owner: OwnerId,
    record_id: Uuid,
    request: &ValidatedSubmit,
) -> Result<NewDelivery, StorageError> {
    let delivery_id = Uuid::now_v7();
    let enqueue_idempotency_key = format!("delivery:{delivery_id}");
    let delivery_payload = DeliveryJobPayload {
        version: DELIVERY_PAYLOAD_VERSION,
        delivery_id,
        owner_id: owner.as_uuid(),
        record_id,
        record_generation: request.expected_generation,
        payload: request.payload.clone(),
    };
    let enqueue_payload = serde_json::to_value(&delivery_payload)
        .map_err(|_| StorageError::Invariant("delivery payload encoding failed"))?;
    let provider_payload = provider_request(&delivery_payload);
    let provider_key = provider_payload.idempotency_key();
    Ok(NewDelivery {
        delivery_id,
        enqueue_idempotency_key,
        enqueue_payload,
        provider_key,
        provider_payload,
    })
}

async fn prepare_submission(
    transaction: &mut PgScopedSql<'_>,
    owner: OwnerId,
    record_id: Uuid,
    request: &ValidatedSubmit,
) -> Result<PreparedSubmission, CommandFailure> {
    let new = prepare_new_delivery(owner, record_id, request)?;
    let inserted = insert_command_identity(
        transaction,
        owner,
        record_id,
        request,
        new.delivery_id,
        &new.enqueue_idempotency_key,
        &new.enqueue_payload,
    )
    .await?;

    if !inserted {
        let retained = load_command(transaction, owner, &request.idempotency_key)
            .await?
            .ok_or(StorageError::Invariant(
                "conflicting command identity disappeared",
            ))?;
        if retained.record_id != record_id
            || retained.expected_generation != request.expected_generation
            || retained.request_payload != request.payload
        {
            return Err(SubmitRejection::IdempotencyConflict.into());
        }
        return Ok(PreparedSubmission::Replayed(SubmitResult {
            disposition: SubmitDisposition::Replayed,
            delivery: retained.delivery()?,
        }));
    }

    let generation: Option<i64> = sqlx::query_scalar(
        "SELECT generation
           FROM reference_records
          WHERE id = $1 AND owner_id = $2
          FOR UPDATE",
    )
    .bind(record_id)
    .bind(owner.as_uuid())
    .fetch_optional(transaction.executor())
    .await
    .map_err(StorageError::from)?;
    let Some(generation) = generation else {
        return Err(SubmitRejection::RecordNotObserved.into());
    };
    if generation != request.expected_generation {
        return Err(SubmitRejection::StaleGeneration {
            current_generation: generation,
        }
        .into());
    }

    Ok(PreparedSubmission::New(Box::new(new), generation))
}

async fn insert_command_identity(
    transaction: &mut PgScopedSql<'_>,
    owner: OwnerId,
    record_id: Uuid,
    request: &ValidatedSubmit,
    delivery_id: Uuid,
    enqueue_idempotency_key: &str,
    enqueue_payload: &Value,
) -> Result<bool, StorageError> {
    let inserted: Option<Uuid> = sqlx::query_scalar(
        "INSERT INTO reference_delivery_commands (
            owner_id, idempotency_key, delivery_id, record_id, expected_generation,
            request_payload, enqueue_job_type, enqueue_idempotency_key, enqueue_payload,
            enqueue_priority, enqueue_max_attempts, enqueue_timeout_seconds, enqueue_stage
         ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
         ON CONFLICT (owner_id, idempotency_key) DO NOTHING
         RETURNING delivery_id",
    )
    .bind(owner.as_uuid())
    .bind(&request.idempotency_key)
    .bind(delivery_id)
    .bind(record_id)
    .bind(request.expected_generation)
    .bind(&request.payload)
    .bind(DELIVERY_JOB_TYPE)
    .bind(enqueue_idempotency_key)
    .bind(enqueue_payload)
    .bind(ENQUEUE_PRIORITY)
    .bind(ENQUEUE_MAX_ATTEMPTS)
    .bind(ENQUEUE_TIMEOUT_SECONDS)
    .bind(JobStage::Queued.as_db_value())
    .fetch_optional(transaction.executor())
    .await
    .map_err(StorageError::from)?;
    Ok(inserted.is_some())
}

async fn insert_delivery(
    transaction: &mut PgScopedSql<'_>,
    owner: OwnerId,
    request: &ValidatedSubmit,
    delivery_id: Uuid,
    outcome: &JobEnqueueOutcome,
    provider_key: &str,
    provider_payload: &ProviderEffectRequest,
) -> Result<(), StorageError> {
    let result = sqlx::query(
        "INSERT INTO reference_deliveries (id, owner_id, idempotency_key, job_id)
         VALUES ($1, $2, $3, $4)",
    )
    .bind(delivery_id)
    .bind(owner.as_uuid())
    .bind(&request.idempotency_key)
    .bind(outcome.job_id)
    .execute(transaction.executor())
    .await?;
    if result.rows_affected() != 1 {
        return Err(StorageError::Invariant("delivery insert changed no row"));
    }
    let effect = sqlx::query(
        "INSERT INTO reference_delivery_effects (
            delivery_id, owner_id, record_id, record_generation,
            provider_key, provider_payload
         ) VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(delivery_id)
    .bind(owner.as_uuid())
    .bind(provider_payload.record_id)
    .bind(provider_payload.record_generation)
    .bind(provider_key)
    .bind(
        serde_json::to_value(provider_payload)
            .map_err(|_| StorageError::Invariant("provider payload encoding failed"))?,
    )
    .execute(transaction.executor())
    .await?;
    if effect.rows_affected() != 1 {
        return Err(StorageError::Invariant(
            "delivery effect insert changed no row",
        ));
    }
    Ok(())
}
