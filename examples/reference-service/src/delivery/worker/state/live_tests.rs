use super::*;
use crate::delivery::{DeliveryService, OwnerId, SubmitDelivery};
use batter::{BoxError, admission::BulkheadCapacity, settings::SecretString};
use runledger_core::jobs::{
    JobContext, JobExecutionError, JobExecutionServices, JobExecutionUpdate,
};
use serde_json::json;
use sqlx::postgres::PgPoolOptions;
use std::time::Instant;

mod confirmation;

async fn with_profiled<T, E>(
    pool: &PgPool,
    work: impl AsyncFnOnce(&runledger_postgres::RunledgerDatabase) -> Result<T, E>,
) -> Result<T, E> {
    let database =
        crate::database::configured((*pool.connect_options()).clone(), pool.options().clone())
            .expect("test profile");
    let result = work(&database).await;
    database.pool().close().await;
    result
}

#[derive(thiserror::Error)]
#[error("provider state probe failed (cause retained)")]
struct Failure(#[source] BoxError);

impl std::fmt::Debug for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self, f)
    }
}

fn finish<T>(body: Result<T, BoxError>, cleanup: Result<(), BoxError>) -> Result<T, BoxError> {
    Ok(batter::test_support::finish(
        body.map_err(Failure),
        cleanup.map_err(Failure),
    )?)
}

struct Services(Instant);

#[tokio::test]
async fn returned_storage_failure_retains_known_provider_delay() -> Result<(), BoxError> {
    let pool =
        PgPoolOptions::new().connect_lazy("postgres://fixture:fixture@127.0.0.1:9/fixture")?;
    let worker = worker(&pool)?;
    pool.close().await;
    let context = inert_context();
    let services = Services(Instant::now() + Duration::from_secs(10));
    let payload = inert_payload();
    let failure = worker
        .record_provider_undispatched(
            JobExecution::new(&context, &services),
            &payload,
            Duration::from_secs(60),
        )
        .await;
    assert_eq!(failure.code, CODE_STATE_UNAVAILABLE);
    assert_eq!(
        failure.retry_timing(),
        Some(runledger_core::jobs::JobRetryTiming::After(
            Duration::from_secs(60)
        ))
    );
    Ok(())
}

fn inert_context() -> JobContext {
    JobContext {
        job_id: Uuid::from_u128(1),
        run_number: 1,
        attempt: 1,
        worker_id: "fixture-worker".into(),
        organization_id: None,
        checkpoint: None,
    }
}

fn inert_payload() -> DeliveryJobPayload {
    DeliveryJobPayload {
        version: DELIVERY_PAYLOAD_VERSION,
        delivery_id: Uuid::from_u128(2),
        owner_id: Uuid::from_u128(3),
        record_id: Uuid::from_u128(4),
        record_generation: 1,
        payload: json!({}),
    }
}

#[tokio::test]
async fn terminal_writers_reject_terminal_sources_before_sql_acquisition() -> Result<(), BoxError> {
    let pool =
        PgPoolOptions::new().connect_lazy("postgres://fixture:fixture@127.0.0.1:9/fixture")?;
    let worker = worker(&pool)?;
    pool.close().await;
    let context = inert_context();
    let services = Services(Instant::now() + Duration::from_secs(10));
    let execution = JobExecution::new(&context, &services);
    let payload = inert_payload();
    for (source, expected) in [
        (ProviderEffectState::Confirmed, "invariant"),
        (ProviderEffectState::BusinessDenied, "invariant"),
        (ProviderEffectState::ManualResolution, "invariant"),
        (ProviderEffectState::Exhausted, "invariant"),
        (ProviderEffectState::AwaitingAttempt, "storage"),
        (ProviderEffectState::RetryableUndispatched, "storage"),
        (ProviderEffectState::ReconcileNeeded, "storage"),
    ] {
        let manual = worker
            .record_manual(execution, &payload, "guard_probe", None, false, source)
            .await;
        let denied = worker
            .record_business_denial(execution, &payload, "guard_probe", source)
            .await;
        for (writer, result) in [("manual", manual), ("denial", denied)] {
            let actual = match result {
                Err(StateError::Invariant) => "invariant",
                Err(StateError::Storage) => "storage",
                other => panic!("unexpected {writer} result for {source:?}: {other:?}"),
            };
            assert_eq!(actual, expected, "{writer} source={source:?}");
        }
    }
    Ok(())
}

#[async_trait]
impl JobExecutionServices for Services {
    fn deadline(&self) -> Instant {
        self.0
    }
    fn remaining_budget(&self) -> Duration {
        self.0.saturating_duration_since(Instant::now())
    }
    async fn persist_progress(&self, _: JobExecutionUpdate<'_>) -> Result<(), JobExecutionError> {
        unreachable!("these probes exercise application SQL, not native progress")
    }
}

#[tokio::test]
#[ignore = "requires an explicitly selected disposable PostgreSQL 18 endpoint"]
async fn provider_state_lock_and_retry_boundaries() -> Result<(), Failure> {
    run_boundaries().await.map_err(Failure)
}

async fn run_boundaries() -> Result<(), BoxError> {
    let url = std::env::var("POSTGRES_TEST_ADMIN_URL")?;
    let config = postgres_test_harness::HarnessConfig::new("batter_state")?
        .with_admin_database_url(&url)
        .with_cleanup_on_start(false)
        .with_connection_budget(8)?
        .with_connections_per_database(4)?;
    let harness = postgres_test_harness::PostgresHarness::start(config).await?;
    let body = async {
        let lease = harness.empty_database().await?;
        let pool = PgPoolOptions::new()
            .max_connections(4)
            .connect(lease.database_url())
            .await;
        let body = match pool {
            Ok(pool) => {
                let probe_pool = pool.clone();
                let result = tokio::spawn(async move { probes(&probe_pool).await })
                    .await
                    .map_err(|error| Box::new(error) as BoxError)
                    .and_then(|result| result);
                pool.close().await;
                result
            }
            Err(error) => Err(Box::new(error) as BoxError),
        };
        finish(body, lease.cleanup().await.map_err(Into::into))?;
        Ok::<_, BoxError>(())
    }
    .await;
    let drained = harness.drain_deferred_cleanup().await;
    let shutdown = harness.shutdown().await;
    finish(
        body,
        batter::test_support::finish(drained, shutdown).map_err(Into::into),
    )?;
    Ok(())
}

async fn probes(pool: &PgPool) -> Result<(), BoxError> {
    with_profiled(pool, async |database| {
        crate::schema::initialize_schema(database).await
    })
    .await?;
    let worker = worker(pool)?;
    for effect_lock in [false, true] {
        lock_only_expiry(&worker, effect_lock).await?;
    }
    durable_eligibility(&worker).await?;
    stale_terminal_sources(&worker).await?;
    confirmation::ordering(&worker).await?;
    Ok(())
}

async fn stale_terminal_sources(worker: &DeliveryWorker) -> Result<(), BoxError> {
    for retained in [
        ProviderEffectState::Confirmed,
        ProviderEffectState::BusinessDenied,
        ProviderEffectState::ManualResolution,
        ProviderEffectState::Exhausted,
        ProviderEffectState::RetryableUndispatched,
    ] {
        let (context, payload) = claimed(&worker.pool).await?;
        let services = Services(Instant::now() + Duration::from_secs(10));
        let execution = JobExecution::new(&context, &services);
        // Model another settled writer after this caller read AwaitingAttempt.
        // Preserve a provider identity and uncertainty to detect collateral loss.
        let before: Value = sqlx::query_scalar(
            "UPDATE reference_delivery_effects e SET state = $1,
                provider_effect_id = 'fixture-retained', acceptance_possible = true
              WHERE delivery_id = $2 RETURNING to_jsonb(e)",
        )
        .bind(retained.database_name())
        .bind(payload.delivery_id)
        .fetch_one(&worker.pool)
        .await?;
        let manual = worker
            .record_manual(
                execution,
                &payload,
                "stale_probe",
                None,
                false,
                ProviderEffectState::AwaitingAttempt,
            )
            .await;
        let denied = worker
            .record_business_denial(
                execution,
                &payload,
                "stale_probe",
                ProviderEffectState::AwaitingAttempt,
            )
            .await;
        assert!(
            matches!(manual, Err(StateError::Invariant)),
            "{retained:?}: {manual:?}"
        );
        assert!(
            matches!(denied, Err(StateError::Invariant)),
            "{retained:?}: {denied:?}"
        );
        let after: Value = sqlx::query_scalar(
            "SELECT to_jsonb(e) FROM reference_delivery_effects e WHERE delivery_id = $1",
        )
        .bind(payload.delivery_id)
        .fetch_one(&worker.pool)
        .await?;
        assert_eq!(after, before, "stale source changed retained {retained:?}");
    }
    Ok(())
}

fn worker(pool: &PgPool) -> Result<DeliveryWorker, BoxError> {
    Ok(DeliveryWorker::new(
        pool.clone(),
        ProviderClient::new(
            url::Url::parse("http://127.0.0.1:9/")?,
            SecretString::new("fixture-token"),
        )?,
        Bulkhead::new(BulkheadCapacity::new(1)?),
    ))
}

async fn claimed(pool: &PgPool) -> Result<(JobContext, DeliveryJobPayload), BoxError> {
    let owner = OwnerId::new(Uuid::now_v7())?;
    let record = Uuid::now_v7();
    sqlx::query("INSERT INTO reference_records (id, owner_id, generation) VALUES ($1, $2, 1)")
        .bind(record)
        .bind(owner.as_uuid())
        .execute(pool)
        .await?;
    let command = with_profiled(pool, async |database| {
        DeliveryService::new(database.clone())
            .submit(
                &batter::operation::OperationOwner::new(Duration::from_secs(5))
                    .map(|owner| owner.into_context())
                    .expect("static budget"),
                owner,
                record,
                SubmitDelivery {
                    expected_generation: 1,
                    idempotency_key: record.to_string(),
                    payload: json!({"scenario":"state-boundary"}),
                },
            )
            .await
    })
    .await?;
    let (job_id, value): (Uuid, Value) = sqlx::query_as(
        "SELECT d.job_id, c.enqueue_payload FROM reference_deliveries d
          JOIN reference_delivery_commands c ON c.delivery_id = d.id WHERE d.id = $1",
    )
    .bind(command.delivery.delivery_id)
    .fetch_one(pool)
    .await?;
    let (run_number, attempt): (i32, i32) = sqlx::query_as(
        "UPDATE job_queue SET status = 'LEASED', attempt = 1, worker_id = 'boundary-worker',
            lease_expires_at = clock_timestamp() + interval '30 seconds',
            last_heartbeat_at = clock_timestamp() WHERE id = $1 RETURNING run_number, attempt",
    )
    .bind(job_id)
    .fetch_one(pool)
    .await?;
    Ok((
        JobContext {
            job_id,
            run_number,
            attempt,
            worker_id: "boundary-worker".into(),
            organization_id: None,
            checkpoint: None,
        },
        serde_json::from_value(value)?,
    ))
}

async fn lock_only_expiry(worker: &DeliveryWorker, effect_lock: bool) -> Result<(), BoxError> {
    let (context, payload) = claimed(&worker.pool).await?;
    let services = Services(Instant::now() + Duration::from_secs(10));
    sqlx::query("UPDATE job_queue SET lease_expires_at = clock_timestamp() + interval '2 seconds' WHERE id = $1")
        .bind(context.job_id).execute(&worker.pool).await?;
    let mut blocker = worker.pool.begin().await?;
    let (query, id, waiting_query) = if effect_lock {
        (
            "SELECT delivery_id FROM reference_delivery_effects WHERE delivery_id = $1 FOR UPDATE",
            payload.delivery_id,
            "%UPDATE reference_delivery_effects%",
        )
    } else {
        (
            "SELECT id FROM job_queue WHERE id = $1 FOR UPDATE",
            context.job_id,
            "%SELECT id FROM job_queue WHERE id = $1 FOR UPDATE%",
        )
    };
    // Both query alternatives above are static, controlled test SQL.
    sqlx::query(sqlx::AssertSqlSafe(query))
        .bind(id)
        .execute(&mut *blocker)
        .await?;
    let mutation = worker.record_business_denial(
        JobExecution::new(&context, &services),
        &payload,
        "fixture_denial",
        ProviderEffectState::AwaitingAttempt,
    );
    let release = async {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let waiting: bool = sqlx::query_scalar(
                    "SELECT EXISTS (SELECT 1 FROM pg_stat_activity WHERE datname = current_database()
                        AND pid <> pg_backend_pid() AND wait_event_type = 'Lock' AND query LIKE $1)",
                ).bind(waiting_query).fetch_one(&worker.pool).await?;
                if waiting { break; }
                tokio::task::yield_now().await;
            }
            let live: bool = sqlx::query_scalar("SELECT lease_expires_at > clock_timestamp() FROM job_queue WHERE id = $1")
                .bind(context.job_id).fetch_one(&worker.pool).await?;
            assert!(live, "must witness the blocked mutation before expiry");
            loop {
                let expired: bool = sqlx::query_scalar("SELECT lease_expires_at <= clock_timestamp() FROM job_queue WHERE id = $1")
                    .bind(context.job_id).fetch_one(&worker.pool).await?;
                if expired { break; }
                tokio::task::yield_now().await;
            }
            Ok::<_, BoxError>(())
        }).await??;
        blocker.commit().await?;
        Ok::<_, BoxError>(())
    };
    let (mutation, released) = tokio::join!(mutation, release);
    released?;
    assert!(
        matches!(mutation, Err(StateError::LeaseLost)),
        "expired mutation: {mutation:?}"
    );
    let state: String =
        sqlx::query_scalar("SELECT state FROM reference_delivery_effects WHERE delivery_id = $1")
            .bind(payload.delivery_id)
            .fetch_one(&worker.pool)
            .await?;
    assert_eq!(
        state, "AWAITING_ATTEMPT",
        "failed authority check must roll back effect writes"
    );
    Ok(())
}

async fn durable_eligibility(worker: &DeliveryWorker) -> Result<(), BoxError> {
    let (context, payload) = claimed(&worker.pool).await?;
    let services = Services(Instant::now() + Duration::from_secs(10));
    let execution = JobExecution::new(&context, &services);
    assert!(
        worker
            .mark_dispatch_possible(execution, &payload, ProviderEffectState::AwaitingAttempt)
            .await
            .unwrap()
    );
    let failure = worker
        .record_provider_undispatched(execution, &payload, Duration::from_secs(60))
        .await;
    assert_eq!(
        failure.retry_timing(),
        Some(runledger_core::jobs::JobRetryTiming::After(
            Duration::from_secs(60)
        ))
    );
    // Drop the native completion value: nothing persists it. The effect commit
    // alone must protect both initial and proven-absence dispatch authorization.
    drop(failure);
    let loaded = worker.load(execution, &payload).await.unwrap();
    assert!(matches!(loaded.plan(1), EffectPlan::Defer(delay) if delay > Duration::from_secs(50)));
    dispatch_eligibility_boundaries(worker, execution, &payload).await
}

async fn dispatch_eligibility_boundaries(
    worker: &DeliveryWorker,
    execution: JobExecution<'_>,
    payload: &DeliveryJobPayload,
) -> Result<(), BoxError> {
    let initial = worker
        .mark_dispatch_possible(
            execution,
            payload,
            ProviderEffectState::RetryableUndispatched,
        )
        .await;
    assert!(
        matches!(initial, Err(StateError::RetryDeferred(delay)) if delay > Duration::from_secs(50))
    );
    sqlx::query(
        "UPDATE reference_delivery_effects SET state = 'RECONCILE_NEEDED',
        acceptance_possible = true, dispatch_possible_at = clock_timestamp(),
        resolve_before = clock_timestamp() + interval '1 hour' WHERE delivery_id = $1",
    )
    .bind(payload.delivery_id)
    .execute(&worker.pool)
    .await?;
    let reconciled = worker
        .renew_dispatch_possible_after_absence(execution, payload)
        .await;
    assert!(
        matches!(reconciled, Err(StateError::RetryDeferred(delay)) if delay > Duration::from_secs(50))
    );
    sqlx::query("UPDATE reference_delivery_effects SET state = 'RETRYABLE_UNDISPATCHED',
        acceptance_possible = false, retry_not_before = clock_timestamp() - interval '1 second' WHERE delivery_id = $1")
        .bind(payload.delivery_id).execute(&worker.pool).await?;
    assert!(
        worker
            .mark_dispatch_possible(
                execution,
                payload,
                ProviderEffectState::RetryableUndispatched
            )
            .await
            .unwrap()
    );
    Ok(())
}
