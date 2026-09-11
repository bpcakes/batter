use super::*;
use batter::operation::{OperationContext, OperationError};
use batter_sqlx::{PgLease, SqlxFailure};
use sqlx::{Connection, PgConnection, PgPool};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

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
        let transaction_active = Arc::new(AtomicBool::new(false));
        let active_in_operation = transaction_active.clone();
        let pool = self.pool.clone();
        let result = context
            .run("delivery.submit", move |scope| async move {
                attempt_submit(
                    &pool,
                    &scope,
                    &active_in_operation,
                    owner,
                    record_id,
                    &request,
                )
                .await
            })
            .await;
        match result {
            Ok(result) => Ok(result),
            Err(OperationError::Interrupted(interruption))
                if transaction_active.load(Ordering::Acquire) =>
            {
                Err(SubmitError::Uncertain(UncertainSubmission::Interrupted(
                    interruption,
                )))
            }
            Err(OperationError::Interrupted(interruption)) => {
                Err(SubmitError::Interrupted(interruption))
            }
            Err(OperationError::Failed(error)) => Err(error.into_submit_error()),
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
        F: for<'a> FnOnce(
            &'a mut PgConnection,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<Output = Result<Option<Delivery>, StorageError>>
                    + Send
                    + 'a,
            >,
        >,
    {
        let pool = self.pool.clone();
        let result = context
            .run("delivery.query", move |scope| async move {
                let mut lease = PgLease::acquire(&pool, &scope)
                    .await
                    .map_err(QueryAttemptError::Acquire)?;
                let result = query(lease.connection())
                    .await
                    .map_err(QueryAttemptError::Storage)?;
                lease.return_to_pool();
                Ok(result)
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
    Acquire(OperationError<SqlxFailure>),
    Begin(sqlx::Error),
    RolledBack(CommandFailure),
    Commit(sqlx::Error),
    Rollback {
        operation: Box<CommandFailure>,
        rollback: sqlx::Error,
    },
}

impl SubmitAttemptError {
    fn into_submit_error(self) -> SubmitError {
        match self {
            Self::Acquire(OperationError::Interrupted(interruption)) => {
                SubmitError::Interrupted(interruption)
            }
            Self::Acquire(OperationError::Failed(error)) => {
                SubmitError::Storage(StorageError::Sqlx(error.into_native()))
            }
            Self::Begin(error) => SubmitError::Storage(StorageError::Sqlx(error)),
            Self::RolledBack(CommandFailure::Rejected(error)) => SubmitError::Rejected(error),
            Self::RolledBack(CommandFailure::Storage(error)) => SubmitError::Storage(error),
            Self::Commit(error) => SubmitError::Uncertain(UncertainSubmission::Commit(error)),
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
    context: &OperationContext,
    transaction_active: &AtomicBool,
    owner: OwnerId,
    record_id: Uuid,
    request: &ValidatedSubmit,
) -> Result<SubmitResult, SubmitAttemptError> {
    let mut lease = PgLease::acquire(pool, context)
        .await
        .map_err(SubmitAttemptError::Acquire)?;
    let mut transaction = lease
        .connection()
        .begin()
        .await
        .map_err(SubmitAttemptError::Begin)?;
    transaction_active.store(true, Ordering::Release);
    let result = submit_in_transaction(&mut transaction, owner, record_id, request).await;
    match result {
        Ok(result) => match transaction.commit().await {
            Ok(()) => {
                transaction_active.store(false, Ordering::Release);
                lease.return_to_pool();
                Ok(result)
            }
            Err(error) => Err(SubmitAttemptError::Commit(error)),
        },
        Err(operation) => match transaction.rollback().await {
            Ok(()) => {
                transaction_active.store(false, Ordering::Release);
                lease.return_to_pool();
                Err(SubmitAttemptError::RolledBack(operation))
            }
            Err(rollback) => Err(SubmitAttemptError::Rollback {
                operation: Box::new(operation),
                rollback,
            }),
        },
    }
}
