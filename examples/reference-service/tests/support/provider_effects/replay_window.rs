use super::*;

pub(super) async fn expired_deadline_probe(
    pool: &PgPool,
    fixture: &ProviderFixture,
    endpoint: &str,
    provider_url: &str,
) -> ProbeResult {
    fixture.set_lookup(super::super::provider::LookupBehavior::Absent);
    fixture.set_dispatch(DispatchBehavior::Accept);
    let before = fixture.counts();
    let expired = submit(pool, 0x41c, "expired-local-window", 14).await?;
    let marked = sqlx::query(
        "UPDATE reference_delivery_effects
            SET state = 'RECONCILE_NEEDED',
                outcome_code = 'dispatch_possible',
                dispatch_possible_at = clock_timestamp() - interval '24 hours 1 second',
                resolve_before = clock_timestamp() - interval '1 second',
                acceptance_possible = true
          WHERE delivery_id = $1 AND state = 'AWAITING_ATTEMPT'",
    )
    .bind(expired.delivery_id)
    .execute(pool)
    .await?;
    if marked.rows_affected() != 1 {
        return Err("local-expiry probe could not install expired uncertainty".into());
    }

    let worker =
        ExecutableChild::start_provider_worker(endpoint, provider_url, "provider-local-expiry")?;
    let observed = async {
        let snapshot = wait_for_snapshot(pool, expired.job_id, Duration::from_secs(5), |snapshot| {
            snapshot.job_status == "DEAD_LETTERED"
                && snapshot.effect_state == "MANUAL_RESOLUTION"
        })
        .await?;
        if snapshot.last_error_code.as_deref() != Some("delivery.manual_resolution_required")
            || snapshot.outcome_code.as_deref() != Some("provider_retention_expired")
            || !snapshot.acceptance_possible
            || snapshot.provider_effect_id.is_some()
        {
            return Err::<_, BoxError>(
                "expired local retention did not preserve unresolved acceptance".into(),
            );
        }
        let after = fixture.counts();
        if after != before {
            return Err(format!(
                "expired local retention crossed the provider boundary: before={before:?}, after={after:?}"
            )
            .into());
        }
        Ok(())
    }
    .await;
    let shutdown = stop(
        worker,
        Signal::Term,
        endpoint.to_owned(),
        provider_url.to_owned(),
    )
    .await;
    finish_results(observed, shutdown)
}

pub(super) async fn probe(
    pool: &PgPool,
    fixture: &ProviderFixture,
    endpoint: &str,
    provider_url: &str,
) -> ProbeResult {
    fixture.set_lookup(super::super::provider::LookupBehavior::Absent);
    fixture.set_dispatch(DispatchBehavior::UnknownText);
    let before = fixture.counts();
    let replay = submit(pool, 0x41b, "renewed-replay-window", 13).await?;
    let marked = sqlx::query(
        "UPDATE reference_delivery_effects
            SET state = 'RECONCILE_NEEDED',
                outcome_code = 'dispatch_possible',
                dispatch_possible_at = clock_timestamp() - interval '23 hours',
                resolve_before = clock_timestamp() + interval '1 hour',
                acceptance_possible = true
          WHERE delivery_id = $1 AND state = 'AWAITING_ATTEMPT'",
    )
    .bind(replay.delivery_id)
    .execute(pool)
    .await?;
    if marked.rows_affected() != 1 {
        return Err("replay-window probe could not install retained uncertainty".into());
    }

    let worker =
        ExecutableChild::start_provider_worker(endpoint, provider_url, "provider-replay-window")?;
    let observed = async {
        let snapshot = wait_for_snapshot(pool, replay.job_id, Duration::from_secs(5), |snapshot| {
            snapshot.job_status == "PENDING"
                && snapshot.effect_state == "RECONCILE_NEEDED"
                && snapshot.attempt == 1
        })
        .await?;
        if snapshot
            .resolution_seconds_remaining
            .is_none_or(|seconds| seconds < 23.0 * 60.0 * 60.0)
        {
            return Err::<_, BoxError>(
                "fresh dispatch after proven absence did not renew provider retention".into(),
            );
        }
        let deferred = sqlx::query(
            "UPDATE job_queue
                SET next_run_at = clock_timestamp() + interval '1 hour'
              WHERE id = $1 AND status::text = 'PENDING'",
        )
        .bind(replay.job_id)
        .execute(pool)
        .await?;
        if deferred.rows_affected() != 1 {
            return Err("renewed replay could not be deferred after observation".into());
        }
        let after = fixture.counts();
        if after.dispatch_requests - before.dispatch_requests != 1
            || after.reconciliation_requests - before.reconciliation_requests != 1
            || after.accepted_effects != before.accepted_effects
        {
            return Err(format!(
                "renewed replay crossed an unexpected provider boundary: before={before:?}, after={after:?}"
            )
            .into());
        }
        Ok(())
    }
    .await;
    let shutdown = stop(
        worker,
        Signal::Term,
        endpoint.to_owned(),
        provider_url.to_owned(),
    )
    .await;
    finish_results(observed, shutdown)
}
