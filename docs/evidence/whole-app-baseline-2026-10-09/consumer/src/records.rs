//! Record persistence: the row-plus-job transaction, reads, and the final-state write.

use batter::operation::{OperationContext, OperationError};
use batter::runledger::native::core::jobs::JobType;
use batter::runledger::native::core::prelude::async_trait;
use batter::runledger::native::postgres::jobs::JobEnqueueIntent;
use batter::runledger::{PgAtomicError, RequiredIntentError, RunledgerDatabase, run_atomic};
use batter::sqlx::{PgQueryHandle, PgScopeError};
use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::Row;
use std::time::Duration;
use uuid::Uuid;

/// The durable job type enqueued for every inserted record.
pub const NOTIFY_JOB: JobType<'static> = JobType::new("records.notify");

/// Payload stored with the `records.notify` intent.
#[derive(Debug, Clone, Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct NotifyPayload {
    pub record_id: Uuid,
}

/// A row as returned by `GET /records/{id}`.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RecordView {
    pub id: Uuid,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub notified_at: Option<DateTime<Utc>>,
}

/// Why the create transaction's body rejected. The runner only reports it
/// after acknowledged rollback.
#[derive(Debug)]
pub enum CreateRejection {
    Insert(PgScopeError<sqlx::Error>),
    Intent(PgScopeError<RequiredIntentError>),
}

impl std::fmt::Display for CreateRejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Insert(_) => "record insert failed",
            Self::Intent(_) => "notification handoff failed",
        })
    }
}

impl std::error::Error for CreateRejection {
    /// The retained scope error is reachable only through the source chain.
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Insert(error) => Some(error),
            Self::Intent(error) => Some(error),
        }
    }
}

/// The complete create outcome: interruption stays distinct from an
/// acknowledged database disposition.
pub type CreateError = OperationError<PgAtomicError<Uuid, CreateRejection>>;

/// A single pooled read or write failed or was interrupted.
pub type QueryError = OperationError<sqlx::Error>;

/// Final-state write capability used by the notification job. The job is
/// generic over it so the handler can be exercised without a database.
#[async_trait]
pub trait NotifiedLedger: Send + Sync {
    async fn mark_notified(&self, context: &OperationContext, id: Uuid) -> Result<(), QueryError>;
}

#[async_trait]
impl<L: NotifiedLedger + ?Sized> NotifiedLedger for &L {
    async fn mark_notified(&self, context: &OperationContext, id: Uuid) -> Result<(), QueryError> {
        (**self).mark_notified(context, id).await
    }
}

#[async_trait]
impl<L: NotifiedLedger + ?Sized> NotifiedLedger for std::sync::Arc<L> {
    async fn mark_notified(&self, context: &OperationContext, id: Uuid) -> Result<(), QueryError> {
        (**self).mark_notified(context, id).await
    }
}

/// Application access to the `records` table through the shared profiled pool.
#[derive(Clone)]
pub struct RecordsStore {
    database: RunledgerDatabase,
    query_budget: Duration,
}

impl RecordsStore {
    /// `query_budget` bounds each single-statement read or write under its caller's context.
    pub fn new(database: RunledgerDatabase, query_budget: Duration) -> Self {
        Self {
            database,
            query_budget,
        }
    }

    /// Insert the row and record the `records.notify` handoff in one
    /// transaction. The id is returned only after the commit is acknowledged.
    #[allow(
        clippy::result_large_err,
        reason = "retain the complete atomic outcome, as the adapter does at this boundary"
    )]
    pub async fn create(
        &self,
        context: &OperationContext,
        name: &str,
    ) -> Result<Uuid, CreateError> {
        let id = Uuid::now_v7();
        let payload = serde_json::to_value(NotifyPayload { record_id: id })
            .expect("a uuid serializes to JSON");
        let key = format!("records:{id}:notify");
        let intent = JobEnqueueIntent::new(NOTIFY_JOB, &payload, &key)
            .with_max_attempts(5)
            .with_timeout_seconds(30);
        context
            .run("records.create", |_| {
                run_atomic(&self.database, async |mut scope| {
                    scope
                        .application(async |sql| {
                            sqlx::query("INSERT INTO records (id, name) VALUES ($1, $2)")
                                .bind(id)
                                .bind(name)
                                .execute(sql.executor())
                                .await
                                .map(|_| ())
                        })
                        .await
                        .map_err(CreateRejection::Insert)?;
                    scope
                        .record_required_job_enqueue_intent(&intent)
                        .await
                        .map_err(CreateRejection::Intent)?;
                    Ok::<Uuid, CreateRejection>(id)
                })
            })
            .await
    }

    /// Read one record, or `None` when absent.
    pub async fn get(
        &self,
        context: &OperationContext,
        id: Uuid,
    ) -> Result<Option<RecordView>, QueryError> {
        let queries = PgQueryHandle::within(
            self.database.pool(),
            context,
            self.query_budget,
            "records.read",
            |error| error,
        )
        .map_err(|error| OperationError::Failed(sqlx::Error::Configuration(error.into())))?;
        queries
            .fetch_optional(
                sqlx::query("SELECT id, name, created_at, notified_at FROM records WHERE id = $1")
                    .bind(id)
                    .try_map(|row: sqlx::postgres::PgRow| {
                        Ok(RecordView {
                            id: row.try_get("id")?,
                            name: row.try_get("name")?,
                            created_at: row.try_get("created_at")?,
                            notified_at: row.try_get("notified_at")?,
                        })
                    }),
            )
            .await
    }
}

#[async_trait]
impl NotifiedLedger for RecordsStore {
    /// Idempotent final-state write: a repeated attempt keeps the first timestamp.
    async fn mark_notified(&self, context: &OperationContext, id: Uuid) -> Result<(), QueryError> {
        let queries = PgQueryHandle::within(
            self.database.pool(),
            context,
            self.query_budget,
            "records.notified",
            |error| error,
        )
        .map_err(|error| OperationError::Failed(sqlx::Error::Configuration(error.into())))?;
        queries
            .execute(
                sqlx::query(
                    "UPDATE records SET notified_at = COALESCE(notified_at, now()) WHERE id = $1",
                )
                .bind(id),
            )
            .await
            .map(|_| ())
    }
}
