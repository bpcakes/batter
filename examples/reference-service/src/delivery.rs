//! Owner-scoped, generation-fenced delivery submission and reconciliation.
//!
//! A submission inserts its command identity, delivery row, and Runledger job
//! in one opaque Batter transaction. Exact replay reads the committed identity;
//! it never starts a second enqueue. Interruption while the transaction disposition
//! remains unknown is reported as uncertain. After commit or rollback acknowledgement,
//! interruption during lease normalization preserves the known application outcome,
//! although the checked-out connection is retired from the pool.
//!
//! ```no_run
//! use batter::operation::OperationContext;
//! use batter_example_reference_service::delivery::{
//!     DeliveryService, OwnerId, ProviderEffectState, SubmitDelivery,
//! };
//! use serde_json::json;
//! use std::time::Duration;
//! use uuid::Uuid;
//!
//! # async fn submit(pool: sqlx::PgPool) -> Result<(), Box<dyn std::error::Error>> {
//! let owner = OwnerId::new(Uuid::from_u128(1))?;
//! let record_id = Uuid::from_u128(2);
//! let context = OperationContext::new(Duration::from_secs(2))?;
//! let service = DeliveryService::new(pool);
//! let accepted = service.submit(&context, owner, record_id, SubmitDelivery {
//!     expected_generation: 3,
//!     idempotency_key: "command-42".into(),
//!     payload: json!({"channel": "example"}),
//! }).await?;
//! assert_eq!(
//!     accepted.delivery.provider.state,
//!     ProviderEffectState::AwaitingAttempt,
//! );
//! assert!(!accepted.delivery.provider.acceptance_possible);
//! let reconciled = service
//!     .get_by_key(&context, owner, "command-42")
//!     .await?;
//! assert_eq!(reconciled.map(|delivery| delivery.delivery_id),
//!     Some(accepted.delivery.delivery_id));
//! # Ok(())
//! # }
//! ```

mod service;
mod worker;
pub(crate) use worker::DeliveryWorker;

use batter::{operation::Interruption, sqlx::PgExecutor};
use runledger_core::jobs::{JobDefinitionSettings, JobSpec, JobStage, JobStatus, JobType};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

use crate::provider::{EFFECT_PROTOCOL_VERSION, ProviderEffectRequest};

/// Durable Runledger type consumed by the reference delivery worker.
pub const DELIVERY_JOB_TYPE: &str = "records.delivery.execute";
/// Maximum retained application idempotency-key size, in UTF-8 bytes.
pub const IDEMPOTENCY_KEY_MAX_BYTES: usize = 128;
/// Maximum meaningful JSON payload size before PostgreSQL acquisition.
pub const PAYLOAD_MAX_BYTES: usize = 16 * 1024;

/// Persisted delivery payload version consumed by the delivery handler.
pub const DELIVERY_PAYLOAD_VERSION: u8 = 1;
const ENQUEUE_PRIORITY: i32 = 0;
const ENQUEUE_MAX_ATTEMPTS: i32 = 3;
const ENQUEUE_TIMEOUT_SECONDS: i32 = 300;

/// Application authority established by authentication, never by a route value.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct OwnerId(Uuid);

impl OwnerId {
    /// Validate a non-nil application owner identifier.
    pub fn new(value: Uuid) -> Result<Self, ValidationError> {
        if value.is_nil() {
            Err(ValidationError::NilOwner)
        } else {
            Ok(Self(value))
        }
    }

    /// Borrow the native UUID used by SQLx and Runledger organization scope.
    #[must_use]
    pub const fn as_uuid(self) -> Uuid {
        self.0
    }
}

/// JSON body accepted by `POST /records/{record_id}/deliveries`.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SubmitDelivery {
    /// Exact record generation the caller intends to deliver.
    pub expected_generation: i64,
    /// Caller-retained identity used for exact replay and reconciliation.
    pub idempotency_key: String,
    /// Meaningful application payload retained and compared canonically.
    pub payload: Value,
}

/// Input validation failures detected before database acquisition.
#[derive(Debug, thiserror::Error)]
pub enum ValidationError {
    /// Authenticated owner identifiers cannot use the nil UUID.
    #[error("owner identifier must not be nil")]
    NilOwner,
    /// Record identifiers cannot use the nil UUID.
    #[error("record identifier must not be nil")]
    NilRecord,
    /// Delivery identifiers cannot use the nil UUID.
    #[error("delivery identifier must not be nil")]
    NilDelivery,
    /// A generation is a positive, monotonically increasing integer.
    #[error("expected generation must be positive")]
    InvalidGeneration,
    /// The key is blank or has surrounding whitespace.
    #[error("idempotency key must be nonblank without surrounding whitespace")]
    InvalidIdempotencyKey,
    /// The UTF-8 key exceeds the retained database bound.
    #[error("idempotency key is too large")]
    IdempotencyKeyTooLarge,
    /// The encoded meaningful payload exceeds the command bound.
    #[error("payload is too large")]
    PayloadTooLarge,
    /// A JSON value could not be encoded for its size check.
    #[error("payload could not be encoded")]
    PayloadEncoding(#[source] serde_json::Error),
}

#[derive(Clone)]
struct ValidatedSubmit {
    expected_generation: i64,
    idempotency_key: String,
    payload: Value,
}

impl SubmitDelivery {
    fn validate(self, record_id: Uuid) -> Result<ValidatedSubmit, ValidationError> {
        if record_id.is_nil() {
            return Err(ValidationError::NilRecord);
        }
        if self.expected_generation <= 0 {
            return Err(ValidationError::InvalidGeneration);
        }
        validate_idempotency_key(&self.idempotency_key)?;
        let encoded =
            serde_json::to_vec(&self.payload).map_err(ValidationError::PayloadEncoding)?;
        if encoded.len() > PAYLOAD_MAX_BYTES {
            return Err(ValidationError::PayloadTooLarge);
        }
        Ok(ValidatedSubmit {
            expected_generation: self.expected_generation,
            idempotency_key: self.idempotency_key,
            payload: self.payload,
        })
    }
}

fn validate_idempotency_key(key: &str) -> Result<(), ValidationError> {
    if key.is_empty()
        || key
            .bytes()
            .any(|byte| !byte.is_ascii_alphanumeric() && !matches!(byte, b'.' | b'_' | b':' | b'-'))
    {
        return Err(ValidationError::InvalidIdempotencyKey);
    }
    if key.len() > IDEMPOTENCY_KEY_MAX_BYTES {
        return Err(ValidationError::IdempotencyKeyTooLarge);
    }
    Ok(())
}

/// Closed application projection of every status in the locked Runledger version.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryState {
    /// Durable and eligible for the worker, but not currently executing.
    Pending,
    /// Currently leased by a worker; completion is not yet known.
    InFlight,
    /// Runledger has durably acknowledged handler success.
    Succeeded,
    /// Runledger exhausted or terminally rejected execution.
    DeadLettered,
    /// Runledger durably cancelled the job.
    Cancelled,
}

impl DeliveryState {
    fn from_job_status(status: JobStatus) -> Self {
        match status {
            JobStatus::Pending => Self::Pending,
            JobStatus::Leased => Self::InFlight,
            JobStatus::Succeeded => Self::Succeeded,
            JobStatus::DeadLettered => Self::DeadLettered,
            JobStatus::Canceled => Self::Cancelled,
        }
    }

    fn from_database(value: &str) -> Result<Self, StorageError> {
        JobStatus::from_db_value(value)
            .map(Self::from_job_status)
            .ok_or(StorageError::Invariant("unknown durable job status"))
    }
}

/// Durable, owner-scoped projection returned by submission and both queries.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Delivery {
    /// Stable delivery identity created by the first command.
    pub delivery_id: Uuid,
    /// Exact target record identity.
    pub record_id: Uuid,
    /// Exact target generation captured by the command.
    pub generation: i64,
    /// Canonical meaningful payload retained with command identity.
    pub payload: Value,
    /// Current state projected from the authoritative Runledger job.
    pub state: DeliveryState,
    /// Retained provider facts, with unresolved native termination projected.
    pub provider: ProviderOutcome,
}

/// Application-owned provider-effect state, separate from native job status.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderEffectState {
    /// No provider mutation has become possible.
    AwaitingAttempt,
    /// A declared outcome proves the latest attempt did not dispatch.
    RetryableUndispatched,
    /// Bytes may have been sent and lookup must resolve the outcome.
    ReconcileNeeded,
    /// The selected protocol confirmed one retained provider effect.
    Confirmed,
    /// Authoritative application or provider policy denied the effect.
    BusinessDenied,
    /// Automatic action is unsafe; an operator must resolve the retained facts.
    ManualResolution,
    /// The native attempt budget ended before a successful resolution.
    Exhausted,
}

impl ProviderEffectState {
    // Read-time projection covers failures for which the handler cannot safely
    // persist another transition (lost lease, unavailable DB, cancellation).
    fn project(self, job: DeliveryState, attempts_spent: bool) -> Self {
        match self {
            Self::Confirmed | Self::BusinessDenied | Self::ManualResolution | Self::Exhausted => {
                self
            }
            Self::AwaitingAttempt | Self::RetryableUndispatched | Self::ReconcileNeeded => {
                match job {
                    DeliveryState::Pending | DeliveryState::InFlight => self,
                    DeliveryState::DeadLettered if attempts_spent => Self::Exhausted,
                    DeliveryState::DeadLettered
                    | DeliveryState::Cancelled
                    | DeliveryState::Succeeded => Self::ManualResolution,
                }
            }
        }
    }

    fn from_database(value: &str) -> Result<Self, StorageError> {
        match value {
            "AWAITING_ATTEMPT" => Ok(Self::AwaitingAttempt),
            "RETRYABLE_UNDISPATCHED" => Ok(Self::RetryableUndispatched),
            "RECONCILE_NEEDED" => Ok(Self::ReconcileNeeded),
            "CONFIRMED" => Ok(Self::Confirmed),
            "BUSINESS_DENIED" => Ok(Self::BusinessDenied),
            "MANUAL_RESOLUTION" => Ok(Self::ManualResolution),
            "EXHAUSTED" => Ok(Self::Exhausted),
            _ => Err(StorageError::Invariant("unknown provider effect state")),
        }
    }

    fn database_name(self) -> &'static str {
        match self {
            Self::AwaitingAttempt => "AWAITING_ATTEMPT",
            Self::RetryableUndispatched => "RETRYABLE_UNDISPATCHED",
            Self::ReconcileNeeded => "RECONCILE_NEEDED",
            Self::Confirmed => "CONFIRMED",
            Self::BusinessDenied => "BUSINESS_DENIED",
            Self::ManualResolution => "MANUAL_RESOLUTION",
            Self::Exhausted => "EXHAUSTED",
        }
    }
}

/// Durable provider facts returned with an owner-scoped delivery.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ProviderOutcome {
    /// Retained effect state, or terminal projection when native work has ended.
    pub state: ProviderEffectState,
    /// Provider identity when a response or lookup supplied one.
    pub provider_effect_id: Option<String>,
    /// Whether an accepted remote effect is still possible but unconfirmed.
    pub acceptance_possible: bool,
}

/// Whether submission created the command or replayed its exact identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SubmitDisposition {
    /// The command and upstream submission were newly committed.
    Accepted,
    /// A previously committed command matched exactly.
    Replayed,
}

/// Successful submission result. Success never implies provider execution.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SubmitResult {
    /// New acceptance or exact replay.
    pub disposition: SubmitDisposition,
    /// Durable identity and current actual state.
    pub delivery: Delivery,
}

/// Concrete business rejections that are safe to distinguish from uncertainty.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum SubmitRejection {
    /// The owner cannot observe the requested record identity.
    #[error("target record was not observed")]
    RecordNotObserved,
    /// The record exists for this owner, but its generation changed.
    #[error("target record generation is stale")]
    StaleGeneration { current_generation: i64 },
    /// The same owner/key was retained with different canonical input.
    #[error("idempotency key conflicts with the retained command")]
    IdempotencyConflict,
}

/// Concrete storage failures retained behind sanitized HTTP responses.
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    /// Native SQLx failure.
    #[error("PostgreSQL operation failed")]
    Sqlx(#[source] sqlx::Error),
    /// Native Runledger failure.
    #[error("durable submission failed")]
    Runledger(#[source] runledger_postgres::Error),
    /// Persisted rows violated an application invariant.
    #[error("persisted delivery invariant failed")]
    Invariant(&'static str),
}

impl From<sqlx::Error> for StorageError {
    fn from(error: sqlx::Error) -> Self {
        Self::Sqlx(error)
    }
}

impl From<runledger_postgres::Error> for StorageError {
    fn from(error: runledger_postgres::Error) -> Self {
        Self::Runledger(error)
    }
}

impl StorageError {
    pub(crate) fn is_pool_unavailable(&self) -> bool {
        matches!(
            self,
            Self::Sqlx(sqlx::Error::PoolClosed | sqlx::Error::PoolTimedOut)
        )
    }
}

/// Why submission cannot assert commit or rollback while disposition is unknown.
#[derive(Debug, thiserror::Error)]
pub enum UncertainSubmission {
    /// PostgreSQL did not acknowledge COMMIT.
    #[error("commit acknowledgement was not received")]
    Commit(#[source] sqlx::Error),
    /// PostgreSQL did not acknowledge rollback of a failed command.
    #[error("rollback acknowledgement was not received")]
    Rollback {
        /// Original command failure retained alongside rollback failure.
        operation: Box<CommandFailure>,
        /// Native rollback failure.
        rollback: sqlx::Error,
    },
    /// The operation boundary stopped polling while the transaction was active,
    /// before commit or rollback acknowledgement.
    #[error("submission was interrupted while transaction disposition was unknown")]
    Interrupted(#[source] Interruption),
}

/// A transaction-body failure retained when rollback acknowledgement is known.
#[derive(Debug, thiserror::Error)]
pub enum CommandFailure {
    /// Concrete business rejection.
    #[error(transparent)]
    Rejected(#[from] SubmitRejection),
    /// Concrete database or durable-runtime failure.
    #[error(transparent)]
    Storage(#[from] StorageError),
}

/// Complete submit failure contract.
#[derive(Debug, thiserror::Error)]
pub enum SubmitError {
    /// Rejected before acquisition.
    #[error("delivery input is invalid")]
    Invalid(#[source] ValidationError),
    /// Business rejection after an acknowledged rollback.
    #[error(transparent)]
    Rejected(#[from] SubmitRejection),
    /// Interrupted before a transaction began.
    #[error(transparent)]
    Interrupted(#[from] Interruption),
    /// Commit/rollback disposition is not acknowledged.
    #[error("delivery submission outcome is uncertain")]
    Uncertain(#[source] UncertainSubmission),
    /// A storage failure whose transaction rollback was acknowledged or never began.
    #[error(transparent)]
    Storage(#[from] StorageError),
}

/// Owner-scoped query failure. `Ok(None)` deliberately hides absence vs ownership.
#[derive(Debug, thiserror::Error)]
pub enum QueryError {
    /// An idempotency key was invalid before acquisition.
    #[error("delivery query input is invalid")]
    Invalid(#[source] ValidationError),
    /// The query deadline/cancellation stopped polling.
    #[error(transparent)]
    Interrupted(#[from] Interruption),
    /// Native storage failure.
    #[error(transparent)]
    Storage(#[from] StorageError),
}

/// Native PostgreSQL command owner; this is not a repository abstraction.
#[derive(Clone)]
pub struct DeliveryService {
    pool: PgPool,
}

/// Immutable, versioned payload stored in the authoritative Runledger job.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct DeliveryJobPayload {
    /// Persisted schema version for compatibility-aware worker decoding.
    pub version: u8,
    /// Stable delivery identity.
    pub delivery_id: Uuid,
    /// Trusted application owner copied into the durable payload.
    pub owner_id: Uuid,
    /// Exact record identity.
    pub record_id: Uuid,
    /// Exact record generation; workers must reject replacement generations.
    pub record_generation: i64,
    /// Meaningful application payload.
    pub payload: Value,
}

fn provider_request(payload: &DeliveryJobPayload) -> ProviderEffectRequest {
    ProviderEffectRequest {
        version: EFFECT_PROTOCOL_VERSION,
        effect_id: payload.delivery_id,
        owner_id: payload.owner_id,
        record_id: payload.record_id,
        record_generation: payload.record_generation,
        payload: payload.payload.clone(),
    }
}

#[derive(FromRow)]
struct CommandRow {
    owner_id: Uuid,
    delivery_id: Uuid,
    record_id: Uuid,
    expected_generation: i64,
    request_payload: Value,
    enqueue_job_type: String,
    enqueue_idempotency_key: String,
    enqueue_payload: Value,
    enqueue_priority: i32,
    enqueue_max_attempts: i32,
    enqueue_timeout_seconds: i32,
    enqueue_stage: String,
    job_id: Option<Uuid>,
    job_status: Option<String>,
    job_type: Option<String>,
    job_organization_id: Option<Uuid>,
    job_idempotency_key: Option<String>,
    job_payload: Option<Value>,
    job_priority: Option<i32>,
    job_max_attempts: Option<i32>,
    job_attempt: Option<i32>,
    job_timeout_seconds: Option<i32>,
    effect_provider_key: Option<String>,
    effect_provider_payload: Option<Value>,
    effect_state: Option<String>,
    effect_provider_effect_id: Option<String>,
    effect_acceptance_possible: Option<bool>,
}

impl CommandRow {
    fn delivery(self) -> Result<Delivery, StorageError> {
        let payload: DeliveryJobPayload = serde_json::from_value(self.enqueue_payload.clone())
            .map_err(|_| StorageError::Invariant("retained enqueue payload is invalid"))?;
        let expected_provider = provider_request(&payload);
        if self.enqueue_job_type != DELIVERY_JOB_TYPE
            || self.enqueue_idempotency_key != format!("delivery:{}", self.delivery_id)
            || self.enqueue_priority != ENQUEUE_PRIORITY
            || self.enqueue_max_attempts != ENQUEUE_MAX_ATTEMPTS
            || self.enqueue_timeout_seconds != ENQUEUE_TIMEOUT_SECONDS
            || self.enqueue_stage != JobStage::Queued.as_db_value()
            || payload.version != DELIVERY_PAYLOAD_VERSION
            || payload.delivery_id != self.delivery_id
            || payload.owner_id != self.owner_id
            || payload.record_id != self.record_id
            || payload.record_generation != self.expected_generation
            || payload.payload != self.request_payload
            || self.effect_provider_key.as_deref()
                != Some(expected_provider.idempotency_key().as_str())
            || self.effect_provider_payload.as_ref()
                != Some(
                    &serde_json::to_value(&expected_provider)
                        .map_err(|_| StorageError::Invariant("provider payload encoding failed"))?,
                )
        {
            return Err(StorageError::Invariant(
                "retained enqueue request does not match delivery identity",
            ));
        }
        let _job_id = self
            .job_id
            .ok_or(StorageError::Invariant("delivery row is missing"))?;
        let status = self
            .job_status
            .ok_or(StorageError::Invariant("upstream job row is missing"))?;
        let effect_state = self
            .effect_state
            .as_deref()
            .ok_or(StorageError::Invariant("delivery effect row is missing"))?;
        let effect_acceptance_possible = self
            .effect_acceptance_possible
            .ok_or(StorageError::Invariant("delivery effect row is missing"))?;
        if self.job_type.as_deref() != Some(self.enqueue_job_type.as_str())
            || self.job_organization_id != Some(self.owner_id)
            || self.job_idempotency_key.as_deref() != Some(self.enqueue_idempotency_key.as_str())
            || self.job_payload.as_ref() != Some(&self.enqueue_payload)
            || self.job_priority != Some(self.enqueue_priority)
            || self.job_max_attempts != Some(self.enqueue_max_attempts)
            || self.job_timeout_seconds != Some(self.enqueue_timeout_seconds)
        {
            return Err(StorageError::Invariant(
                "upstream job does not match retained enqueue request",
            ));
        }
        let state = DeliveryState::from_database(&status)?;
        let attempt = self
            .job_attempt
            .ok_or(StorageError::Invariant("upstream job row is missing"))?;
        Ok(Delivery {
            delivery_id: self.delivery_id,
            record_id: self.record_id,
            generation: self.expected_generation,
            payload: self.request_payload,
            state,
            provider: ProviderOutcome {
                state: ProviderEffectState::from_database(effect_state)?
                    .project(state, attempt >= self.enqueue_max_attempts),
                provider_effect_id: self.effect_provider_effect_id,
                acceptance_possible: effect_acceptance_possible,
            },
        })
    }
}

async fn load_command<E: PgExecutor + ?Sized>(
    connection: &mut E,
    owner: OwnerId,
    idempotency_key: &str,
) -> Result<Option<CommandRow>, StorageError> {
    load_command_at(connection, owner, CommandLookup::Key(idempotency_key)).await
}

enum CommandLookup<'a> {
    Key(&'a str),
    Id(Uuid),
}

// Both owner-scoped reads use one SQL snapshot and one identity/projection check.
async fn load_command_at<E: PgExecutor + ?Sized>(
    connection: &mut E,
    owner: OwnerId,
    lookup: CommandLookup<'_>,
) -> Result<Option<CommandRow>, StorageError> {
    let (idempotency_key, delivery_id) = match lookup {
        CommandLookup::Key(key) => (Some(key), None),
        CommandLookup::Id(id) => (None, Some(id)),
    };
    sqlx::query_as(
        "SELECT c.owner_id, c.delivery_id, c.record_id, c.expected_generation, c.request_payload,
                c.enqueue_job_type, c.enqueue_idempotency_key, c.enqueue_payload,
                c.enqueue_priority, c.enqueue_max_attempts, c.enqueue_timeout_seconds,
                c.enqueue_stage, d.job_id, q.status::text AS job_status,
                q.job_type, q.organization_id AS job_organization_id,
                q.idempotency_key AS job_idempotency_key, q.payload AS job_payload,
                q.priority AS job_priority, q.max_attempts AS job_max_attempts,
                q.attempt AS job_attempt,
                q.timeout_seconds AS job_timeout_seconds,
                e.provider_key AS effect_provider_key,
                e.provider_payload AS effect_provider_payload,
                e.state AS effect_state,
                e.provider_effect_id AS effect_provider_effect_id,
                e.acceptance_possible AS effect_acceptance_possible
           FROM reference_delivery_commands c
           LEFT JOIN reference_deliveries d
             ON d.owner_id = c.owner_id
            AND d.idempotency_key = c.idempotency_key
            AND d.id = c.delivery_id
           LEFT JOIN job_queue q ON q.id = d.job_id
           LEFT JOIN reference_delivery_effects e ON e.delivery_id = d.id
          WHERE c.owner_id = $1
            AND (c.idempotency_key = $2 OR c.delivery_id = $3)",
    )
    .bind(owner.as_uuid())
    .bind(idempotency_key)
    .bind(delivery_id)
    .fetch_optional(connection.executor())
    .await
    .map_err(StorageError::from)
}

async fn load_delivery_by_id<E: PgExecutor + ?Sized>(
    connection: &mut E,
    owner: OwnerId,
    delivery_id: Uuid,
) -> Result<Option<Delivery>, StorageError> {
    load_command_at(connection, owner, CommandLookup::Id(delivery_id))
        .await?
        .map(CommandRow::delivery)
        .transpose()
}

/// Producer definition synchronized before accepting delivery commands.
///
/// The application root registers the matching provider handler separately.
pub fn delivery_job_spec() -> Result<JobSpec, runledger_core::jobs::JobSpecError> {
    JobSpec::new(JobType::new(DELIVERY_JOB_TYPE))?.with_settings(
        JobDefinitionSettings::new()
            .version(1)
            .max_attempts(ENQUEUE_MAX_ATTEMPTS)
            .timeout_seconds(ENQUEUE_TIMEOUT_SECONDS)
            .priority(ENQUEUE_PRIORITY)
            .enabled(true),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn request(key: impl Into<String>, payload: Value) -> SubmitDelivery {
        SubmitDelivery {
            expected_generation: 1,
            idempotency_key: key.into(),
            payload,
        }
    }

    #[test]
    fn validation_rejects_invalid_identity_and_bounds_before_acquisition() {
        assert!(matches!(
            OwnerId::new(Uuid::nil()),
            Err(ValidationError::NilOwner)
        ));
        assert!(matches!(
            request("key", json!({})).validate(Uuid::nil()),
            Err(ValidationError::NilRecord)
        ));
        assert!(matches!(
            request(" key", json!({})).validate(Uuid::now_v7()),
            Err(ValidationError::InvalidIdempotencyKey)
        ));
        assert!(matches!(
            request("x".repeat(IDEMPOTENCY_KEY_MAX_BYTES + 1), json!({})).validate(Uuid::now_v7()),
            Err(ValidationError::IdempotencyKeyTooLarge)
        ));
        assert!(matches!(
            request("key", Value::String("x".repeat(PAYLOAD_MAX_BYTES))).validate(Uuid::now_v7()),
            Err(ValidationError::PayloadTooLarge)
        ));
    }

    #[test]
    fn state_projection_is_closed_over_the_locked_upstream_vocabulary() {
        assert_eq!(
            DeliveryState::from_job_status(JobStatus::Pending),
            DeliveryState::Pending
        );
        assert_eq!(
            DeliveryState::from_job_status(JobStatus::Leased),
            DeliveryState::InFlight
        );
        assert_eq!(
            DeliveryState::from_job_status(JobStatus::Succeeded),
            DeliveryState::Succeeded
        );
        assert_eq!(
            DeliveryState::from_job_status(JobStatus::DeadLettered),
            DeliveryState::DeadLettered
        );
        assert_eq!(
            DeliveryState::from_job_status(JobStatus::Canceled),
            DeliveryState::Cancelled
        );
        assert!(DeliveryState::from_database("NEW_UPSTREAM_STATE").is_err());
    }

    #[test]
    fn provider_effect_projection_matches_the_database_check_vocabulary() {
        let migration = include_str!("../migrations/202609160001_delivery_effect_outcomes.sql");
        let check = migration
            .split("chk_reference_delivery_effects_state")
            .nth(1)
            .unwrap()
            .split(")),")
            .next()
            .unwrap();
        let database: Vec<_> = check.split('\'').skip(1).step_by(2).collect();
        let projected = [
            ProviderEffectState::AwaitingAttempt,
            ProviderEffectState::RetryableUndispatched,
            ProviderEffectState::ReconcileNeeded,
            ProviderEffectState::Confirmed,
            ProviderEffectState::BusinessDenied,
            ProviderEffectState::ManualResolution,
            ProviderEffectState::Exhausted,
        ]
        .map(ProviderEffectState::database_name);
        assert_eq!(database, projected);
    }

    #[test]
    fn public_provider_outcome_json_preserves_state_and_acceptance_truth() {
        let exhausted = serde_json::to_value(ProviderOutcome {
            state: ProviderEffectState::Exhausted,
            provider_effect_id: Some("provider:retained".to_owned()),
            acceptance_possible: true,
        })
        .unwrap();
        assert_eq!(
            exhausted,
            serde_json::json!({
                "state": "exhausted",
                "provider_effect_id": "provider:retained",
                "acceptance_possible": true,
            })
        );

        let manual = serde_json::to_value(ProviderOutcome {
            state: ProviderEffectState::ManualResolution,
            provider_effect_id: None,
            acceptance_possible: false,
        })
        .unwrap();
        assert_eq!(manual["state"], "manual_resolution");
        assert_ne!(manual["state"], exhausted["state"]);
    }

    #[test]
    fn producer_definition_matches_immutable_enqueue_settings() {
        let spec = delivery_job_spec().unwrap();
        assert_eq!(spec.job_type().as_str(), DELIVERY_JOB_TYPE);
        let settings = spec.settings();
        assert_eq!(settings.version, 1);
        assert_eq!(settings.default_priority, ENQUEUE_PRIORITY);
        assert_eq!(settings.max_attempts, ENQUEUE_MAX_ATTEMPTS);
        assert_eq!(settings.default_timeout_seconds, ENQUEUE_TIMEOUT_SECONDS);
        assert!(settings.is_enabled);
    }
}
