use super::*;
use sqlx::{FromRow, Postgres, Transaction};
use uuid::Uuid;

impl DeliveryWorker {
    /// Lock first, then validate time-dependent authority in a fresh statement.
    async fn begin_live_effect(
        &self,
        execution: JobExecution<'_>,
    ) -> Result<LiveEffectTransaction, StateError> {
        let transaction = self.pool.begin().await.map_err(|_| StateError::Storage)?;
        LiveEffectTransaction::lock_job(transaction, execution).await
    }

    pub(super) async fn load(
        &self,
        execution: JobExecution<'_>,
        payload: &DeliveryJobPayload,
    ) -> Result<EffectSnapshot, StateError> {
        let mut live = self.begin_live_effect(execution).await?;
        let row: EffectRow = sqlx::query_as(
            "SELECT e.owner_id, e.record_id, e.record_generation,
                    e.provider_key, e.provider_payload, e.state,
                    e.resolve_before IS NOT NULL
                        AND e.resolve_before > clock_timestamp() AS retention_active,
                    COALESCE(CEIL(EXTRACT(EPOCH FROM
                        (e.retry_not_before - clock_timestamp())) * 1000)::bigint, 0)
                        AS retry_delay_ms,
                    r.generation AS current_generation
               FROM reference_delivery_effects e
               JOIN reference_deliveries d ON d.id = e.delivery_id
          LEFT JOIN reference_records r
                 ON r.id = e.record_id AND r.owner_id = e.owner_id
              WHERE e.delivery_id = $1
                AND d.job_id = $2",
        )
        .bind(payload.delivery_id)
        .bind(live.job_id)
        .fetch_optional(&mut *live.transaction)
        .await
        .map_err(|_| StateError::Storage)?
        .ok_or(StateError::Invariant)?;

        let provider_payload: ProviderEffectRequest =
            serde_json::from_value(row.provider_payload).map_err(|_| StateError::Invariant)?;
        if row.owner_id != payload.owner_id
            || row.record_id != payload.record_id
            || row.record_generation != payload.record_generation
            || provider_payload != provider_request(payload)
            || row.provider_key != provider_payload.idempotency_key()
        {
            return Err(StateError::Invariant);
        }
        let state =
            ProviderEffectState::from_database(&row.state).map_err(|_| StateError::Invariant)?;
        live.commit().await?;
        Ok(EffectSnapshot {
            provider_key: row.provider_key,
            provider_payload,
            state,
            retention_active: row.retention_active,
            retry_delay: remaining_retry_delay(row.retry_delay_ms),
            current_generation: row.current_generation,
        })
    }

    pub(super) async fn mark_dispatch_possible(
        &self,
        execution: JobExecution<'_>,
        payload: &DeliveryJobPayload,
        expected: ProviderEffectState,
    ) -> Result<bool, StateError> {
        if !matches!(
            expected,
            ProviderEffectState::AwaitingAttempt | ProviderEffectState::RetryableUndispatched
        ) {
            return Err(StateError::Invariant);
        }
        self.set_dispatch_possible(execution, payload, expected)
            .await
    }

    pub(super) async fn renew_dispatch_possible_after_absence(
        &self,
        execution: JobExecution<'_>,
        payload: &DeliveryJobPayload,
    ) -> Result<bool, StateError> {
        self.set_dispatch_possible(execution, payload, ProviderEffectState::ReconcileNeeded)
            .await
    }

    async fn set_dispatch_possible(
        &self,
        execution: JobExecution<'_>,
        payload: &DeliveryJobPayload,
        expected: ProviderEffectState,
    ) -> Result<bool, StateError> {
        let retention_seconds =
            i64::try_from(IDEMPOTENCY_RETENTION.as_secs()).map_err(|_| StateError::Invariant)?;
        let mut live = self.begin_live_effect(execution).await?;
        live.require_dispatch_eligible(payload.delivery_id).await?;
        let result = sqlx::query(
            "UPDATE reference_delivery_effects e
                SET state = 'RECONCILE_NEEDED',
                    outcome_code = 'dispatch_possible',
                    dispatch_possible_at = clock_timestamp(),
                    resolve_before = clock_timestamp() + ($3::bigint * interval '1 second'),
                    acceptance_possible = true,
                    updated_at = clock_timestamp()
              WHERE e.delivery_id = $1
                AND e.state = $4
                AND EXISTS (
                    SELECT 1 FROM reference_deliveries d
                     WHERE d.id = e.delivery_id AND d.job_id = $2
                )
                AND EXISTS (
                    SELECT 1 FROM reference_records r
                     WHERE r.id = e.record_id
                       AND r.owner_id = e.owner_id
                       AND r.generation = e.record_generation
                )",
        )
        .bind(payload.delivery_id)
        .bind(live.job_id)
        .bind(retention_seconds)
        .bind(expected.database_name())
        .execute(&mut *live.transaction)
        .await
        .map_err(|_| StateError::Storage)?;
        let marked = result.rows_affected() == 1;
        live.commit().await?;
        Ok(marked)
    }

    pub(super) async fn record_state_invariant(
        &self,
        execution: JobExecution<'_>,
        payload: &DeliveryJobPayload,
    ) -> Result<(), StateError> {
        let mut live = self.begin_live_effect(execution).await?;
        let result = sqlx::query(
            "UPDATE reference_delivery_effects e
                SET state = 'MANUAL_RESOLUTION',
                    outcome_code = 'retained_state_invariant',
                    updated_at = clock_timestamp()
              WHERE e.delivery_id = $1
                AND EXISTS (
                    SELECT 1 FROM reference_deliveries d
                     WHERE d.id = e.delivery_id AND d.job_id = $2
                )
                AND e.state IN (
                    'AWAITING_ATTEMPT',
                    'RETRYABLE_UNDISPATCHED',
                    'RECONCILE_NEEDED'
                )",
        )
        .bind(payload.delivery_id)
        .bind(live.job_id)
        .execute(&mut *live.transaction)
        .await
        .map_err(|_| StateError::Storage)?;
        if result.rows_affected() != 1 {
            return Err(StateError::Invariant);
        }
        live.commit().await
    }

    pub(super) async fn record_admission_undispatched(
        &self,
        execution: JobExecution<'_>,
        payload: &DeliveryJobPayload,
        code: &'static str,
        expected: ProviderEffectState,
    ) -> Result<ProviderEffectState, StateError> {
        if !matches!(
            expected,
            ProviderEffectState::AwaitingAttempt | ProviderEffectState::RetryableUndispatched
        ) {
            return Err(StateError::Invariant);
        }
        self.record_failed_attempt(execution, payload, code, expected, false, Duration::ZERO)
            .await
    }

    /// Persist the outcome and its eligibility together, then construct the
    /// native scheduling result. Callers cannot detach timing from persistence.
    pub(super) async fn record_provider_undispatched(
        &self,
        execution: JobExecution<'_>,
        payload: &DeliveryJobPayload,
        delay: Duration,
    ) -> JobFailure {
        match self
            .record_failed_attempt(
                execution,
                payload,
                CODE_PROVIDER_UNDISPATCHED,
                ProviderEffectState::ReconcileNeeded,
                false,
                delay,
            )
            .await
        {
            Ok(state) => retry_or_exhausted(
                state,
                CODE_PROVIDER_UNDISPATCHED,
                "The provider declared that no effect was dispatched.",
                Some(delay),
            ),
            Err(error) => state_failure(error).retry_not_before_delay(delay),
        }
    }

    pub(super) async fn record_uncertain_failure(
        &self,
        execution: JobExecution<'_>,
        payload: &DeliveryJobPayload,
        code: &'static str,
    ) -> Result<ProviderEffectState, StateError> {
        self.record_failed_attempt(
            execution,
            payload,
            code,
            ProviderEffectState::ReconcileNeeded,
            true,
            Duration::ZERO,
        )
        .await
    }

    async fn record_failed_attempt(
        &self,
        execution: JobExecution<'_>,
        payload: &DeliveryJobPayload,
        code: &'static str,
        expected: ProviderEffectState,
        acceptance_possible: bool,
        retry_delay: Duration,
    ) -> Result<ProviderEffectState, StateError> {
        let legal_source = if acceptance_possible {
            expected == ProviderEffectState::ReconcileNeeded
        } else {
            matches!(
                expected,
                ProviderEffectState::AwaitingAttempt
                    | ProviderEffectState::RetryableUndispatched
                    | ProviderEffectState::ReconcileNeeded
            )
        };
        if !legal_source {
            return Err(StateError::Invariant);
        }
        let mut live = self.begin_live_effect(execution).await?;
        let retry_delay_ms =
            i64::try_from(retry_delay.as_millis()).map_err(|_| StateError::Invariant)?;
        let state: String = sqlx::query_scalar(
            "UPDATE reference_delivery_effects e
                SET state = CASE WHEN q.attempt >= q.max_attempts
                                 THEN 'EXHAUSTED'
                                 WHEN $1 THEN 'RECONCILE_NEEDED'
                                 ELSE 'RETRYABLE_UNDISPATCHED' END,
                    outcome_code = $2,
                    dispatch_possible_at = CASE WHEN $1 THEN e.dispatch_possible_at ELSE NULL END,
                    resolve_before = CASE WHEN $1 THEN e.resolve_before ELSE NULL END,
                    acceptance_possible = $1,
                    retry_not_before = GREATEST(e.retry_not_before,
                        clock_timestamp() + ($6::bigint * interval '1 millisecond')),
                    updated_at = clock_timestamp()
               FROM job_queue q
              WHERE e.delivery_id = $3
                AND q.id = $4
                AND EXISTS (
                    SELECT 1 FROM reference_deliveries d
                     WHERE d.id = e.delivery_id AND d.job_id = q.id
                )
                AND e.state = $5
          RETURNING e.state",
        )
        .bind(acceptance_possible)
        .bind(code)
        .bind(payload.delivery_id)
        .bind(live.job_id)
        .bind(expected.database_name())
        .bind(retry_delay_ms)
        .fetch_optional(&mut *live.transaction)
        .await
        .map_err(|_| StateError::Storage)?
        .ok_or(StateError::Invariant)?;
        let state =
            ProviderEffectState::from_database(&state).map_err(|_| StateError::Invariant)?;
        live.commit().await?;
        Ok(state)
    }

    pub(super) async fn record_business_denial(
        &self,
        execution: JobExecution<'_>,
        payload: &DeliveryJobPayload,
        code: &'static str,
        expected: ProviderEffectState,
    ) -> Result<(), StateError> {
        self.record_terminal(
            execution,
            payload,
            TerminalUpdate {
                state: "BUSINESS_DENIED",
                code,
                provider_effect_id: None,
                acceptance_possible: false,
                expected: expected.try_into()?,
            },
        )
        .await
    }

    pub(super) async fn record_manual(
        &self,
        execution: JobExecution<'_>,
        payload: &DeliveryJobPayload,
        code: &'static str,
        provider_effect_id: Option<&str>,
        acceptance_possible: bool,
        expected: ProviderEffectState,
    ) -> Result<(), StateError> {
        self.record_terminal(
            execution,
            payload,
            TerminalUpdate {
                state: "MANUAL_RESOLUTION",
                code,
                provider_effect_id,
                acceptance_possible,
                expected: expected.try_into()?,
            },
        )
        .await
    }

    async fn record_terminal(
        &self,
        execution: JobExecution<'_>,
        payload: &DeliveryJobPayload,
        update: TerminalUpdate<'_>,
    ) -> Result<(), StateError> {
        let mut live = self.begin_live_effect(execution).await?;
        let result = sqlx::query(
            "UPDATE reference_delivery_effects e
                SET state = $1,
                    outcome_code = $2,
                    provider_effect_id = COALESCE($3, e.provider_effect_id),
                    acceptance_possible = $4,
                    updated_at = clock_timestamp()
              WHERE e.delivery_id = $5
                AND EXISTS (
                    SELECT 1 FROM reference_deliveries d
                     WHERE d.id = e.delivery_id AND d.job_id = $6
                )
                AND e.state = $7",
        )
        .bind(update.state)
        .bind(update.code)
        .bind(update.provider_effect_id)
        .bind(update.acceptance_possible)
        .bind(payload.delivery_id)
        .bind(live.job_id)
        .bind(update.expected.retained().database_name())
        .execute(&mut *live.transaction)
        .await
        .map_err(|_| StateError::Storage)?;
        if result.rows_affected() != 1 {
            return Err(StateError::Invariant);
        }
        live.commit().await
    }

    pub(super) async fn confirm(
        &self,
        execution: JobExecution<'_>,
        payload: &DeliveryJobPayload,
        provider_effect_id: String,
    ) -> Result<JobCompletion, JobFailure> {
        let mut live = self
            .begin_confirmation(execution, payload)
            .await
            .map_err(state_failure)?;
        let state: String = sqlx::query_scalar(
            "UPDATE reference_delivery_effects e
                SET state = CASE WHEN EXISTS (
                        SELECT 1 FROM reference_records r
                         WHERE r.id = e.record_id
                           AND r.owner_id = e.owner_id
                           AND r.generation = e.record_generation
                    ) THEN 'CONFIRMED' ELSE 'MANUAL_RESOLUTION' END,
                    provider_effect_id = $1,
                    outcome_code = CASE WHEN EXISTS (
                        SELECT 1 FROM reference_records r
                         WHERE r.id = e.record_id
                           AND r.owner_id = e.owner_id
                           AND r.generation = e.record_generation
                    ) THEN 'provider_confirmed' ELSE 'target_changed_after_dispatch' END,
                    acceptance_possible = false,
                    updated_at = clock_timestamp()
              WHERE e.delivery_id = $2
                AND EXISTS (
                    SELECT 1 FROM reference_deliveries d
                     WHERE d.id = e.delivery_id AND d.job_id = $3
                )
                AND e.state = 'RECONCILE_NEEDED'
          RETURNING e.state",
        )
        .bind(provider_effect_id)
        .bind(payload.delivery_id)
        .bind(live.job_id)
        .fetch_optional(&mut *live.transaction)
        .await
        .map_err(|_| state_failure(StateError::Storage))?
        .ok_or_else(|| state_failure(StateError::Invariant))?;
        let state = ProviderEffectState::from_database(&state)
            .map_err(|_| state_failure(StateError::Invariant))?;
        live.commit().await.map_err(state_failure)?;
        match state {
            ProviderEffectState::Confirmed => Ok(JobCompletion::success()),
            ProviderEffectState::ManualResolution => Err(JobFailure::terminal(
                CODE_MANUAL_RESOLUTION,
                "The target changed after provider acceptance.",
            )),
            _ => Err(state_failure(StateError::Invariant)),
        }
    }

    async fn begin_confirmation(
        &self,
        execution: JobExecution<'_>,
        payload: &DeliveryJobPayload,
    ) -> Result<LiveEffectTransaction, StateError> {
        let mut transaction = self.pool.begin().await.map_err(|_| StateError::Storage)?;
        // Record -> job -> effect. Submission also locks the record before
        // enqueue. Waiting for replacement must not hold the heartbeat's job lock.
        // FOR SHARE conflicts with generation updates, unlike FOR KEY SHARE.
        let locked: Option<Uuid> = sqlx::query_scalar(
            "SELECT id FROM reference_records WHERE id = $1 AND owner_id = $2 FOR SHARE",
        )
        .bind(payload.record_id)
        .bind(payload.owner_id)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|_| StateError::Storage)?;
        if locked.is_none() {
            return Err(StateError::Invariant);
        }
        // The confirmation UPDATE gets a fresh snapshot after the record lock
        // wait, and the guard remains held through the lease recheck and commit.
        LiveEffectTransaction::lock_job(transaction, execution).await
    }
}

struct LiveEffectTransaction {
    transaction: Transaction<'static, Postgres>,
    job_id: Uuid,
    run_number: i32,
    attempt: i32,
    worker_id: String,
}

impl LiveEffectTransaction {
    async fn lock_job(
        mut transaction: Transaction<'static, Postgres>,
        execution: JobExecution<'_>,
    ) -> Result<Self, StateError> {
        let locked: Option<Uuid> =
            sqlx::query_scalar("SELECT id FROM job_queue WHERE id = $1 FOR UPDATE")
                .bind(execution.context().job_id)
                .fetch_optional(&mut *transaction)
                .await
                .map_err(|_| StateError::Storage)?;
        if locked.is_none() {
            return Err(StateError::LeaseLost);
        }
        let mut live = Self {
            transaction,
            job_id: execution.context().job_id,
            run_number: execution.context().run_number,
            attempt: execution.context().attempt,
            worker_id: execution.context().worker_id.clone(),
        };
        live.validate_lease().await?;
        Ok(live)
    }

    async fn validate_lease(&mut self) -> Result<(), StateError> {
        let live: bool = sqlx::query_scalar(
            "SELECT EXISTS (
                SELECT 1 FROM job_queue WHERE id = $1 AND run_number = $2
                 AND attempt = $3 AND worker_id = $4 AND status::text = 'LEASED'
                 AND lease_expires_at > clock_timestamp()
            )",
        )
        .bind(self.job_id)
        .bind(self.run_number)
        .bind(self.attempt)
        .bind(&self.worker_id)
        .fetch_one(&mut *self.transaction)
        .await
        .map_err(|_| StateError::Storage)?;
        if live {
            Ok(())
        } else {
            Err(StateError::LeaseLost)
        }
    }

    async fn require_dispatch_eligible(&mut self, delivery_id: Uuid) -> Result<(), StateError> {
        // Serialize with effect writers before evaluating the time predicate.
        let locked: Option<Uuid> = sqlx::query_scalar(
            "SELECT delivery_id FROM reference_delivery_effects WHERE delivery_id = $1 FOR UPDATE",
        )
        .bind(delivery_id)
        .fetch_optional(&mut *self.transaction)
        .await
        .map_err(|_| StateError::Storage)?;
        if locked.is_none() {
            return Err(StateError::Invariant);
        }
        let remaining: i64 = sqlx::query_scalar(
            "SELECT COALESCE(CEIL(EXTRACT(EPOCH FROM
                (retry_not_before - clock_timestamp())) * 1000)::bigint, 0)
               FROM reference_delivery_effects WHERE delivery_id = $1",
        )
        .bind(delivery_id)
        .fetch_one(&mut *self.transaction)
        .await
        .map_err(|_| StateError::Storage)?;
        let delay = remaining_retry_delay(remaining);
        if delay.is_zero() {
            Ok(())
        } else {
            Err(StateError::RetryDeferred(delay))
        }
    }

    async fn commit(mut self) -> Result<(), StateError> {
        // Effect-row/constraint lock waits may also outlive the original check.
        // A failed final check rolls back the unpublished effect mutation.
        self.validate_lease().await?;
        self.transaction
            .commit()
            .await
            .map_err(|_| StateError::Storage)
    }
}

#[derive(FromRow)]
struct EffectRow {
    owner_id: Uuid,
    record_id: Uuid,
    record_generation: i64,
    provider_key: String,
    provider_payload: Value,
    state: String,
    retention_active: bool,
    retry_delay_ms: i64,
    current_generation: Option<i64>,
}

struct TerminalUpdate<'a> {
    state: &'static str,
    code: &'static str,
    provider_effect_id: Option<&'a str>,
    acceptance_possible: bool,
    expected: UnresolvedEffectState,
}

/// A terminal write can consume unresolved truth, never overwrite terminal truth.
/// Keep this private to the persistence boundary so callers cannot skip validation.
enum UnresolvedEffectState {
    AwaitingAttempt,
    RetryableUndispatched,
    ReconcileNeeded,
}

impl TryFrom<ProviderEffectState> for UnresolvedEffectState {
    type Error = StateError;

    fn try_from(state: ProviderEffectState) -> Result<Self, Self::Error> {
        match state {
            ProviderEffectState::AwaitingAttempt => Ok(Self::AwaitingAttempt),
            ProviderEffectState::RetryableUndispatched => Ok(Self::RetryableUndispatched),
            ProviderEffectState::ReconcileNeeded => Ok(Self::ReconcileNeeded),
            ProviderEffectState::Confirmed
            | ProviderEffectState::BusinessDenied
            | ProviderEffectState::ManualResolution
            | ProviderEffectState::Exhausted => Err(StateError::Invariant),
        }
    }
}

impl UnresolvedEffectState {
    fn retained(self) -> ProviderEffectState {
        match self {
            Self::AwaitingAttempt => ProviderEffectState::AwaitingAttempt,
            Self::RetryableUndispatched => ProviderEffectState::RetryableUndispatched,
            Self::ReconcileNeeded => ProviderEffectState::ReconcileNeeded,
        }
    }
}

#[cfg(test)]
mod live_tests;
