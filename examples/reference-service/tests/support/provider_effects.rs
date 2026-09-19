use super::{
    ProbeResult,
    fixture_diagnostics::ProbeError,
    provider::{DispatchBehavior, ProviderFixture},
    startup_process::{self, ExecutableChild, Signal},
};
use batter::{BoxError, operation::OperationContext};
use batter_example_reference_service::{
    delivery::{DeliveryService, OwnerId, SubmitDelivery},
    schema::initialize_schema,
};
use serde_json::json;
use sqlx::{FromRow, PgPool, types::Uuid};
use std::time::Duration;

mod lease_fence;
mod replay_window;
mod retained_boundaries;
mod retry_eligibility;

const OWNER: u128 = 0x401;
const RECORD: u128 = 0x402;
#[derive(FromRow)]
struct Snapshot {
    job_status: String,
    last_error_code: Option<String>,
    attempt: i32,
    attempt_rows: i64,
    effect_state: String,
    outcome_code: Option<String>,
    provider_effect_id: Option<String>,
    acceptance_possible: bool,
    resolution_deadline_retained: bool,
    resolution_seconds_remaining: Option<f64>,
}
struct Submitted {
    delivery_id: Uuid,
    job_id: Uuid,
}
/// Accept one provider effect, kill the real worker before its response, then
/// require ordinary process restart to reconcile without another mutation.
pub async fn crash_and_restart(pool: PgPool) -> ProbeResult {
    initialize_schema(&pool).await?;
    let submitted = submit(&pool, RECORD, "provider-crash-window", 1).await?;
    let delivery_id = submitted.delivery_id;
    let job_id = submitted.job_id;
    let fixture = ProviderFixture::start(DispatchBehavior::AcceptBehindBarrier).await?;
    let provider_url = fixture.base_url();
    let endpoint = startup_process::endpoint(&pool);
    let first =
        ExecutableChild::start_provider_worker(&endpoint, &provider_url, "provider-crash-first")?;
    let initial = async {
        fixture.wait_for_acceptance(Duration::from_secs(8)).await?;
        let uncertain = snapshot(&pool, job_id).await?;
        if uncertain.job_status != "LEASED"
            || uncertain.attempt != 1
            || uncertain.effect_state != "RECONCILE_NEEDED"
            || !uncertain.acceptance_possible
            || uncertain.provider_effect_id.is_some()
        {
            return Err::<_, BoxError>(
                "provider acceptance was not retained as an in-flight uncertainty".into(),
            );
        }
        Ok(())
    }
    .await;
    if let Err(error) = initial {
        let endpoint_for_abort = endpoint.clone();
        let provider_for_abort = provider_url.clone();
        let diagnostics = tokio::task::spawn_blocking(move || {
            let forbidden = startup_process::provider_forbidden_values(
                &endpoint_for_abort,
                &provider_for_abort,
            );
            first.abort_diagnostics(&forbidden)
        })
        .await
        .map_err(|join_error| format!("provider diagnostic task failed: {join_error}"));
        fixture.release_response();
        let process_result = match diagnostics {
            Ok(diagnostics) => Err(format!("{error}; {diagnostics}").into()),
            Err(diagnostics_error) => Err(format!("{error}; {diagnostics_error}").into()),
        };
        let close = fixture.close().await;
        return finish_results(process_result, close);
    }

    let crash = crash(first, endpoint.clone(), provider_url.clone()).await;
    fixture.release_response();
    if let Err(error) = crash {
        let close = fixture.close().await;
        return finish_results(Err(error), close);
    }

    let second =
        ExecutableChild::start_provider_worker(&endpoint, &provider_url, "provider-crash-second")?;
    let observed = async {
        let completed = wait_for_snapshot(&pool, job_id, Duration::from_secs(10), |snapshot| {
            snapshot.job_status == "SUCCEEDED" && snapshot.effect_state == "CONFIRMED"
        })
        .await?;
        if completed.attempt != 2
            || completed.attempt_rows != 2
            || completed.acceptance_possible
            || completed.provider_effect_id.as_deref() != Some(&format!("provider:{delivery_id}"))
        {
            return Err("restart did not retain the expected two-attempt confirmation".into());
        }
        let counts = fixture.counts();
        if counts.dispatch_requests != 1
            || counts.accepted_effects != 1
            || counts.reconciliation_requests == 0
        {
            return Err(format!("unexpected provider request counts: {counts:?}").into());
        }
        Ok(())
    }
    .await;

    let process_result = match observed {
        Ok(()) => {
            replacement_between_attempts(&pool, &fixture, &endpoint, &provider_url, second).await
        }
        Err(error) => {
            let shutdown = stop(second, Signal::Term, endpoint.clone(), provider_url.clone()).await;
            finish_results(Err(error), shutdown)
        }
    };
    let provider_result = fixture.close().await;
    finish_results(process_result, provider_result)
}

async fn replacement_between_attempts(
    pool: &PgPool,
    fixture: &ProviderFixture,
    endpoint: &str,
    provider_url: &str,
    worker: ExecutableChild,
) -> ProbeResult {
    fixture.set_dispatch(DispatchBehavior::AcceptBehindBarrier);
    fixture.set_lookup(super::provider::LookupBehavior::Stored);
    let before = fixture.counts();
    let prepared = async {
        let replacement = submit(pool, 0x403, "replacement-between-attempts", 12).await?;
        fixture
            .wait_for_accepted_count(before.accepted_effects + 1, Duration::from_secs(5))
            .await?;
        let uncertain = snapshot(pool, replacement.job_id).await?;
        if uncertain.job_status != "LEASED"
            || uncertain.effect_state != "RECONCILE_NEEDED"
            || !uncertain.acceptance_possible
        {
            return Err::<_, BoxError>(
                "replacement scenario did not retain its accepted uncertainty".into(),
            );
        }
        Ok(replacement)
    }
    .await;
    let replacement = match prepared {
        Ok(replacement) => replacement,
        Err(error) => {
            fixture.set_dispatch(DispatchBehavior::Accept);
            fixture.release_response();
            let shutdown = stop(
                worker,
                Signal::Term,
                endpoint.to_owned(),
                provider_url.to_owned(),
            )
            .await;
            return finish_results(Err(error), shutdown);
        }
    };

    let crash = crash(worker, endpoint.to_owned(), provider_url.to_owned()).await;
    fixture.release_response();
    crash?;
    let changed =
        sqlx::query("UPDATE reference_records SET generation = 2 WHERE id = $1 AND generation = 1")
            .bind(Uuid::from_u128(0x403))
            .execute(pool)
            .await?;
    if changed.rows_affected() != 1 {
        return Err("cross-attempt generation replacement did not occur".into());
    }
    let restarted = ExecutableChild::start_provider_worker(
        endpoint,
        provider_url,
        "provider-replacement-restart",
    )?;
    let observed = async {
        let completed = wait_for_snapshot(
            pool,
            replacement.job_id,
            Duration::from_secs(10),
            |snapshot| {
                snapshot.job_status == "DEAD_LETTERED"
                    && snapshot.effect_state == "MANUAL_RESOLUTION"
            },
        )
        .await?;
        if completed.attempt != 2
            || completed.attempt_rows != 2
            || completed.acceptance_possible
            || completed.provider_effect_id.as_deref()
                != Some(&format!("provider:{}", replacement.delivery_id))
        {
            return Err("cross-attempt replacement did not retain accepted effect truth".into());
        }
        let after = fixture.counts();
        if after.dispatch_requests - before.dispatch_requests != 1
            || after.accepted_effects - before.accepted_effects != 1
            || after.reconciliation_requests - before.reconciliation_requests != 1
        {
            return Err(format!(
                "cross-attempt replacement bypassed lookup-before-resolution: before={before:?}, after={after:?}"
            )
            .into());
        }
        Ok(())
    }
    .await;
    let shutdown = stop(
        restarted,
        Signal::Term,
        endpoint.to_owned(),
        provider_url.to_owned(),
    )
    .await;
    finish_results(observed, shutdown)
}

/// Exercise admission ordering and every application-owned provider projection
/// without treating ordinary job failures as process-critical failures.
pub async fn outcome_contracts(pool: PgPool) -> ProbeResult {
    initialize_schema(&pool).await?;
    let fixture = ProviderFixture::start(DispatchBehavior::AcceptBehindBarrier).await?;
    let body = outcome_body(&pool, &fixture).await;
    let cleanup = fixture.close().await;
    finish_results(body, cleanup)
}

async fn outcome_body(pool: &PgPool, fixture: &ProviderFixture) -> ProbeResult {
    let endpoint = startup_process::endpoint(pool);
    let provider_url = fixture.base_url();

    retained_boundaries::identity(pool, fixture, &endpoint, &provider_url).await?;
    retained_boundaries::reconciliation(pool, fixture, &endpoint, &provider_url).await?;
    replay_window::expired_deadline_probe(pool, fixture, &endpoint, &provider_url).await?;
    replay_window::probe(pool, fixture, &endpoint, &provider_url).await?;
    lease_fence::probe(pool, fixture, &endpoint, &provider_url).await?;
    retry_eligibility::probe(pool, fixture, &endpoint, &provider_url).await?;
    ordinary_outcome_phase(pool, fixture, &endpoint, &provider_url).await?;
    expiry_phase(pool, fixture, &endpoint, &provider_url).await?;
    admission_phase(pool, fixture, &endpoint, &provider_url).await
}

async fn admission_phase(
    pool: &PgPool,
    fixture: &ProviderFixture,
    endpoint: &str,
    provider_url: &str,
) -> ProbeResult {
    fixture.set_dispatch(DispatchBehavior::AcceptBehindBarrier);
    fixture.set_lookup(super::provider::LookupBehavior::Stored);
    let before = fixture.counts();
    let first = submit(pool, 0x410, "admission-first", 1).await?;
    let second = submit(pool, 0x411, "admission-second", 2).await?;
    let admission_worker =
        ExecutableChild::start_admission_worker(endpoint, provider_url, "provider-admission")?;
    let before_release = wait_for_admission_wait(pool, &first, &second).await;
    fixture.set_dispatch(DispatchBehavior::Accept);
    fixture.release_response();
    let observed = async {
        before_release?;
        for submitted in [&first, &second] {
            wait_for_snapshot(pool, submitted.job_id, Duration::from_secs(5), |snapshot| {
                snapshot.job_status == "SUCCEEDED" && snapshot.effect_state == "CONFIRMED"
            })
            .await?;
        }
        Ok::<_, BoxError>(())
    }
    .await;
    let shutdown = stop(
        admission_worker,
        Signal::Term,
        endpoint.to_owned(),
        provider_url.to_owned(),
    )
    .await;
    finish_results(observed, shutdown)?;
    let after = fixture.counts();
    if after.dispatch_requests - before.dispatch_requests != 2
        || after.accepted_effects - before.accepted_effects != 2
    {
        return Err(format!(
            "bounded provider admission did not serialize both requests: before={before:?}, after={after:?}"
        )
        .into());
    }

    uncertain_and_terminal_admission(pool, fixture, endpoint, provider_url).await?;
    fixture.set_dispatch(DispatchBehavior::Accept);
    fixture.verify_identity_contract().await
}

async fn uncertain_and_terminal_admission(
    pool: &PgPool,
    fixture: &ProviderFixture,
    endpoint: &str,
    provider_url: &str,
) -> ProbeResult {
    fixture.set_dispatch(DispatchBehavior::AcceptBehindBarrier);
    let before = fixture.counts();
    let blocker = submit(pool, 0x418, "admission-blocker", 9).await?;
    let uncertain = submit(pool, 0x419, "admission-uncertain", 10).await?;
    let terminal = submit(pool, 0x41a, "terminal-redelivery", 11).await?;
    prepare_deferred_admission_states(pool, &uncertain, &terminal).await?;

    let worker = ExecutableChild::start_admission_worker(
        endpoint,
        provider_url,
        "provider-admission-state",
    )?;
    let observed = async {
        fixture
            .wait_for_accepted_count(before.accepted_effects + 1, Duration::from_secs(5))
            .await?;
        activate_deferred(pool, &[uncertain.job_id, terminal.job_id]).await?;

        let terminal_snapshot =
            wait_for_snapshot(pool, terminal.job_id, Duration::from_secs(5), |snapshot| {
                snapshot.job_status == "SUCCEEDED" && snapshot.effect_state == "CONFIRMED"
            })
            .await?;
        if terminal_snapshot.attempt != 1
            || terminal_snapshot.provider_effect_id.as_deref()
                != Some("provider:terminal-redelivery")
        {
            return Err::<_, BoxError>(
                "terminal redelivery did not complete idempotently before admission".into(),
            );
        }

        let uncertain_snapshot =
            wait_for_snapshot(pool, uncertain.job_id, Duration::from_secs(5), |snapshot| {
                snapshot.job_status == "PENDING"
                    && snapshot.effect_state == "RECONCILE_NEEDED"
                    && snapshot.attempt == 1
            })
            .await?;
        if !uncertain_snapshot.acceptance_possible
            || !uncertain_snapshot.resolution_deadline_retained
            || uncertain_snapshot.provider_effect_id.is_some()
            || uncertain_snapshot.outcome_code.as_deref() != Some("delivery.admission_interrupted")
        {
            return Err::<_, BoxError>(
                "admission interruption lost retained uncertainty or its diagnostic".into(),
            );
        }
        Ok(())
    }
    .await;

    fixture.set_dispatch(DispatchBehavior::Accept);
    fixture.release_response();
    let observed = match observed {
        Ok(()) => wait_for_snapshot(pool, blocker.job_id, Duration::from_secs(5), |snapshot| {
            snapshot.job_status == "SUCCEEDED" && snapshot.effect_state == "CONFIRMED"
        })
        .await
        .map(|_| ()),
        Err(error) => Err(error),
    };
    let shutdown = stop(
        worker,
        Signal::Term,
        endpoint.to_owned(),
        provider_url.to_owned(),
    )
    .await;
    finish_results(observed, shutdown)?;

    let after = fixture.counts();
    if after.dispatch_requests - before.dispatch_requests != 1
        || after.accepted_effects - before.accepted_effects != 1
        || after.reconciliation_requests != before.reconciliation_requests
    {
        return Err(format!(
            "waiting or terminal work crossed the provider boundary: before={before:?}, after={after:?}"
        )
        .into());
    }
    Ok(())
}

async fn ordinary_outcome_phase(
    pool: &PgPool,
    fixture: &ProviderFixture,
    endpoint: &str,
    provider_url: &str,
) -> ProbeResult {
    fixture.set_dispatch(DispatchBehavior::BusinessDenied);
    let business = submit(pool, 0x412, "business-denial", 3).await?;
    let worker =
        ExecutableChild::start_provider_worker(endpoint, provider_url, "provider-outcomes")?;
    let observed = observe_initial_outcomes(pool, fixture, business).await;
    fixture.release_response();
    let observed = match observed {
        Ok((replacement, accepted_before)) => {
            complete_replacement(pool, fixture, replacement, accepted_before).await
        }
        Err(error) => Err(error),
    };
    let shutdown = stop(
        worker,
        Signal::Term,
        endpoint.to_owned(),
        provider_url.to_owned(),
    )
    .await;
    finish_results(observed, shutdown)
}

async fn observe_initial_outcomes(
    pool: &PgPool,
    fixture: &ProviderFixture,
    business: Submitted,
) -> Result<(Submitted, usize), BoxError> {
    let business = wait_for_snapshot(pool, business.job_id, Duration::from_secs(5), |snapshot| {
        snapshot.job_status == "SUCCEEDED" && snapshot.effect_state == "BUSINESS_DENIED"
    })
    .await?;
    if business.acceptance_possible || business.provider_effect_id.is_some() {
        return Err("business denial retained an uncertain provider acceptance".into());
    }

    fixture.set_dispatch(DispatchBehavior::RateLimited {
        retry_after_ms: 24 * 60 * 60 * 1_000,
    });
    let known = submit(pool, 0x417, "known-undispatched", 8).await?;
    let known = wait_for_snapshot(pool, known.job_id, Duration::from_secs(5), |snapshot| {
        snapshot.job_status == "PENDING"
            && snapshot.effect_state == "RETRYABLE_UNDISPATCHED"
            && snapshot.attempt == 1
    })
    .await?;
    if known.acceptance_possible || known.provider_effect_id.is_some() {
        return Err("known-undispatched response retained possible acceptance".into());
    }

    fixture.set_dispatch(DispatchBehavior::UnknownText);
    fixture.set_lookup(super::provider::LookupBehavior::Absent);
    let before = fixture.counts();
    let exhausted = submit(pool, 0x413, "opaque-provider-text", 4).await?;
    // The locked native policy owns its unqualified retry delays (roughly ten
    // then twenty seconds here); the application does not install a test-only
    // override for an indeterminate provider result.
    let exhausted = wait_for_snapshot(
        pool,
        exhausted.job_id,
        Duration::from_secs(45),
        |snapshot| snapshot.job_status == "DEAD_LETTERED" && snapshot.effect_state == "EXHAUSTED",
    )
    .await?;
    if exhausted.attempt != 3
        || exhausted.attempt_rows != 3
        || exhausted.last_error_code.as_deref() != Some("delivery.attempts_exhausted")
        || !exhausted.acceptance_possible
        || !exhausted.resolution_deadline_retained
    {
        return Err("opaque responses did not consume the bounded three-attempt budget".into());
    }
    let after = fixture.counts();
    if after.dispatch_requests - before.dispatch_requests != 3
        || after.reconciliation_requests - before.reconciliation_requests != 2
        || after.accepted_effects != before.accepted_effects
    {
        return Err(format!(
            "opaque text authorized replay without the required absent lookups: before={before:?}, after={after:?}"
        )
        .into());
    }

    fixture.set_dispatch(DispatchBehavior::AcceptBehindBarrier);
    fixture.set_lookup(super::provider::LookupBehavior::Stored);
    let accepted_before = fixture.counts().accepted_effects;
    let replacement = submit(pool, 0x414, "generation-replacement", 5).await?;
    fixture
        .wait_for_accepted_count(accepted_before + 1, Duration::from_secs(5))
        .await?;
    let changed =
        sqlx::query("UPDATE reference_records SET generation = 2 WHERE id = $1 AND generation = 1")
            .bind(Uuid::from_u128(0x414))
            .execute(pool)
            .await?;
    if changed.rows_affected() != 1 {
        return Err("authoritative generation replacement did not occur".into());
    }
    Ok((replacement, accepted_before))
}

async fn complete_replacement(
    pool: &PgPool,
    fixture: &ProviderFixture,
    replacement: Submitted,
    accepted_before: usize,
) -> ProbeResult {
    let snapshot = wait_for_snapshot(
        pool,
        replacement.job_id,
        Duration::from_secs(5),
        |snapshot| {
            snapshot.job_status == "DEAD_LETTERED" && snapshot.effect_state == "MANUAL_RESOLUTION"
        },
    )
    .await?;
    if snapshot.provider_effect_id.as_deref()
        != Some(&format!("provider:{}", replacement.delivery_id))
        || snapshot.acceptance_possible
        || fixture.counts().accepted_effects != accepted_before + 1
    {
        return Err("replacement generation mutated the accepted effect projection".into());
    }
    Ok(())
}

async fn expiry_phase(
    pool: &PgPool,
    fixture: &ProviderFixture,
    endpoint: &str,
    provider_url: &str,
) -> ProbeResult {
    fixture.set_dispatch(DispatchBehavior::Accept);
    fixture.set_lookup(super::provider::LookupBehavior::Expired);
    let expired = submit(pool, 0x415, "retention-expiry", 6).await?;
    let marked = sqlx::query(
        "UPDATE reference_delivery_effects
            SET state = 'RECONCILE_NEEDED',
                outcome_code = 'dispatch_possible',
                dispatch_possible_at = clock_timestamp(),
                resolve_before = clock_timestamp() + interval '1 hour',
                acceptance_possible = true
          WHERE delivery_id = $1 AND state = 'AWAITING_ATTEMPT'",
    )
    .bind(expired.delivery_id)
    .execute(pool)
    .await?;
    if marked.rows_affected() != 1 {
        return Err("retention probe could not install its pre-dispatch uncertainty".into());
    }
    let worker = ExecutableChild::start_provider_worker(endpoint, provider_url, "provider-expiry")?;
    let observed = async {
        let expired = wait_for_snapshot(pool, expired.job_id, Duration::from_secs(5), |snapshot| {
            snapshot.job_status == "DEAD_LETTERED" && snapshot.effect_state == "MANUAL_RESOLUTION"
        })
        .await?;
        if !expired.acceptance_possible || expired.provider_effect_id.is_some() {
            return Err("retention expiry did not preserve unresolved acceptance".into());
        }

        // A terminal job outcome must not drain the native process. Processing a
        // later business denial is the externally visible witness.
        fixture.set_lookup(super::provider::LookupBehavior::Stored);
        fixture.set_dispatch(DispatchBehavior::BusinessDenied);
        let later = submit(pool, 0x416, "after-terminal-outcome", 7).await?;
        wait_for_snapshot(pool, later.job_id, Duration::from_secs(5), |snapshot| {
            snapshot.job_status == "SUCCEEDED" && snapshot.effect_state == "BUSINESS_DENIED"
        })
        .await?;
        Ok(())
    }
    .await;
    let shutdown = stop(
        worker,
        Signal::Int,
        endpoint.to_owned(),
        provider_url.to_owned(),
    )
    .await;
    finish_results(observed, shutdown)
}

async fn submit(pool: &PgPool, record: u128, key: &str, value: i64) -> Result<Submitted, BoxError> {
    let owner = OwnerId::new(Uuid::from_u128(OWNER))?;
    let record_id = Uuid::from_u128(record);
    sqlx::query("INSERT INTO reference_records (id, owner_id, generation) VALUES ($1, $2, 1)")
        .bind(record_id)
        .bind(owner.as_uuid())
        .execute(pool)
        .await?;
    let context = OperationContext::new(Duration::from_secs(5))?;
    let result = DeliveryService::new(pool.clone())
        .submit(
            &context,
            owner,
            record_id,
            SubmitDelivery {
                expected_generation: 1,
                idempotency_key: key.to_owned(),
                payload: json!({"scenario": key, "value": value}),
            },
        )
        .await?;
    let job_id = sqlx::query_scalar("SELECT job_id FROM reference_deliveries WHERE id = $1")
        .bind(result.delivery.delivery_id)
        .fetch_one(pool)
        .await?;
    Ok(Submitted {
        delivery_id: result.delivery.delivery_id,
        job_id,
    })
}

async fn prepare_deferred_admission_states(
    pool: &PgPool,
    uncertain: &Submitted,
    terminal: &Submitted,
) -> ProbeResult {
    let deferred = sqlx::query(
        "UPDATE job_queue
            SET next_run_at = clock_timestamp() + interval '1 hour'
          WHERE id = ANY($1) AND status::text = 'PENDING'",
    )
    .bind(&[uncertain.job_id, terminal.job_id][..])
    .execute(pool)
    .await?;
    if deferred.rows_affected() != 2 {
        return Err("admission states could not be deferred before worker startup".into());
    }

    let uncertain_marked = sqlx::query(
        "UPDATE reference_delivery_effects
            SET state = 'RECONCILE_NEEDED',
                outcome_code = 'dispatch_possible',
                dispatch_possible_at = clock_timestamp(),
                resolve_before = clock_timestamp() + interval '1 hour',
                acceptance_possible = true
          WHERE delivery_id = $1 AND state = 'AWAITING_ATTEMPT'",
    )
    .bind(uncertain.delivery_id)
    .execute(pool)
    .await?;
    let timeout = sqlx::query("UPDATE job_queue SET timeout_seconds = 1 WHERE id = $1")
        .bind(uncertain.job_id)
        .execute(pool)
        .await?;
    let terminal_marked = sqlx::query(
        "UPDATE reference_delivery_effects
            SET state = 'CONFIRMED',
                outcome_code = 'provider_confirmed',
                provider_effect_id = 'provider:terminal-redelivery',
                acceptance_possible = false
          WHERE delivery_id = $1 AND state = 'AWAITING_ATTEMPT'",
    )
    .bind(terminal.delivery_id)
    .execute(pool)
    .await?;
    if uncertain_marked.rows_affected() != 1
        || timeout.rows_affected() != 1
        || terminal_marked.rows_affected() != 1
    {
        return Err("admission state fixtures did not install exactly once".into());
    }
    Ok(())
}

async fn activate_deferred(pool: &PgPool, jobs: &[Uuid]) -> ProbeResult {
    let activated = sqlx::query(
        "UPDATE job_queue
            SET next_run_at = clock_timestamp()
          WHERE id = ANY($1) AND status::text = 'PENDING'",
    )
    .bind(jobs)
    .execute(pool)
    .await?;
    if activated.rows_affected() != jobs.len() as u64 {
        return Err("deferred admission states were not activated together".into());
    }
    Ok(())
}

async fn wait_for_admission_wait(
    pool: &PgPool,
    first: &Submitted,
    second: &Submitted,
) -> ProbeResult {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let left = snapshot(pool, first.job_id).await?;
            let right = snapshot(pool, second.job_id).await?;
            let states = [&left, &right];
            let dispatching = states.iter().any(|snapshot| {
                snapshot.job_status == "LEASED" && snapshot.effect_state == "RECONCILE_NEEDED"
            });
            let waiting = states.iter().any(|snapshot| {
                snapshot.job_status == "LEASED"
                    && snapshot.effect_state == "AWAITING_ATTEMPT"
                    && snapshot.attempt == 1
            });
            if dispatching && waiting {
                return Ok::<_, sqlx::Error>(());
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .map_err(|_| -> BoxError { "bounded provider admission wait was not observed".into() })?
    .map_err(|error| Box::new(error) as BoxError)
}

async fn stop(
    child: ExecutableChild,
    signal: Signal,
    endpoint: String,
    provider_url: String,
) -> ProbeResult {
    tokio::task::spawn_blocking(move || {
        let forbidden = startup_process::provider_forbidden_values(&endpoint, &provider_url);
        child.stop(signal, &forbidden)
    })
    .await?
    .map_err(|error| error.into())
}

async fn crash(child: ExecutableChild, endpoint: String, provider_url: String) -> ProbeResult {
    tokio::task::spawn_blocking(move || {
        let forbidden = startup_process::provider_forbidden_values(&endpoint, &provider_url);
        child.crash(&forbidden)
    })
    .await?
    .map_err(|error| error.into())
}

async fn wait_for_snapshot(
    pool: &PgPool,
    job_id: Uuid,
    limit: Duration,
    accepted: impl Fn(&Snapshot) -> bool,
) -> Result<Snapshot, BoxError> {
    tokio::time::timeout(limit, async {
        loop {
            let current = snapshot(pool, job_id).await?;
            if accepted(&current) {
                return Ok::<_, sqlx::Error>(current);
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .map_err(|_| -> BoxError { "durable provider state did not settle before its bound".into() })?
    .map_err(|error| Box::new(error) as BoxError)
}

async fn snapshot(pool: &PgPool, job_id: Uuid) -> Result<Snapshot, sqlx::Error> {
    sqlx::query_as(
        "SELECT q.status::text AS job_status,
                q.last_error_code,
                q.attempt,
                (SELECT count(*) FROM job_attempts a WHERE a.job_id = q.id) AS attempt_rows,
                e.state AS effect_state,
                e.outcome_code,
                e.provider_effect_id,
                e.acceptance_possible,
                e.resolve_before IS NOT NULL AS resolution_deadline_retained,
                EXTRACT(EPOCH FROM (e.resolve_before - clock_timestamp()))::float8
                    AS resolution_seconds_remaining
           FROM job_queue q
           JOIN reference_deliveries d ON d.job_id = q.id
           JOIN reference_delivery_effects e ON e.delivery_id = d.id
          WHERE q.id = $1",
    )
    .bind(job_id)
    .fetch_one(pool)
    .await
}

fn finish_results(body: ProbeResult, cleanup: ProbeResult) -> ProbeResult {
    batter::test_support::finish(
        body.map_err(ProbeError::new),
        cleanup.map_err(ProbeError::new),
    )
    .map_err(|error| Box::new(error) as BoxError)
}
