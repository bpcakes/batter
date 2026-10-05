use super::*;

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
        fixture
            .wait_for_acceptance(EXTERNAL_EFFECT_ALLOWANCE)
            .await?;
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
        let close = fixture.close_within(EXTERNAL_EFFECT_ALLOWANCE).await;
        return finish_results(process_result, close);
    }

    let crash = crash(first, endpoint.clone(), provider_url.clone()).await;
    fixture.release_response();
    if let Err(error) = crash {
        let close = fixture.close_within(EXTERNAL_EFFECT_ALLOWANCE).await;
        return finish_results(Err(error), close);
    }

    let second =
        ExecutableChild::start_provider_worker(&endpoint, &provider_url, "provider-crash-second")?;
    let observed = async {
        let completed = wait_for_snapshot(&pool, job_id, EXTERNAL_EFFECT_ALLOWANCE, |snapshot| {
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
    let provider_result = fixture.close_within(EXTERNAL_EFFECT_ALLOWANCE).await;
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
    fixture.set_lookup(super::super::provider::LookupBehavior::Stored);
    let before = fixture.counts();
    let prepared = async {
        let replacement = submit(pool, 0x403, "replacement-between-attempts", 12).await?;
        fixture
            .wait_for_accepted_count(before.accepted_effects + 1, EXTERNAL_EFFECT_ALLOWANCE)
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
            EXTERNAL_EFFECT_ALLOWANCE,
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
