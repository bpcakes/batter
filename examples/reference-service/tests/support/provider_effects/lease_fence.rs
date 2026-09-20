use super::*;

/// Hold the authoritative job row while an accepted response reaches the
/// handler, then revoke the native lease before releasing the row lock. The
/// application transition must not confirm through the stale claim.
pub(super) async fn probe(
    pool: &PgPool,
    fixture: &ProviderFixture,
    endpoint: &str,
    provider_url: &str,
) -> ProbeResult {
    fixture.set_dispatch(DispatchBehavior::AcceptBehindBarrier);
    fixture.set_lookup(super::super::provider::LookupBehavior::Stored);
    let before = fixture.counts();
    let submitted = submit(pool, 0x41d, "stale-lease-fence", 15).await?;
    sqlx::query("UPDATE job_queue SET attempt = max_attempts - 1 WHERE id = $1")
        .bind(submitted.job_id)
        .execute(pool)
        .await?;
    let worker =
        ExecutableChild::start_provider_worker(endpoint, provider_url, "provider-lease-fence")?;

    let observed = async {
        fixture
            .wait_for_accepted_count(before.accepted_effects + 1, EXTERNAL_EFFECT_ALLOWANCE)
            .await?;
        let uncertain = snapshot(pool, submitted.job_id).await?;
        if uncertain.job_status != "LEASED"
            || uncertain.effect_state != "RECONCILE_NEEDED"
            || !uncertain.acceptance_possible
        {
            return Err::<_, BoxError>(
                "lease-fence probe did not reach retained dispatch uncertainty".into(),
            );
        }

        let mut transaction = pool.begin().await?;
        let locked: Option<Uuid> = sqlx::query_scalar(
            "SELECT id FROM job_queue WHERE id = $1 AND status::text = 'LEASED' FOR UPDATE",
        )
        .bind(submitted.job_id)
        .fetch_optional(&mut *transaction)
        .await?;
        if locked != Some(submitted.job_id) {
            return Err("lease-fence probe could not lock the active native lease".into());
        }

        fixture.release_response();
        wait_for_effect_fence_wait(pool).await?;
        let revoked = sqlx::query(
            "UPDATE job_queue
                SET status = 'DEAD_LETTERED',
                    worker_id = NULL,
                    lease_expires_at = NULL,
                    last_heartbeat_at = NULL,
                    next_run_at = clock_timestamp() + interval '1 hour',
                    updated_at = clock_timestamp()
              WHERE id = $1 AND status::text = 'LEASED'",
        )
        .bind(submitted.job_id)
        .execute(&mut *transaction)
        .await?;
        if revoked.rows_affected() != 1 {
            return Err("lease-fence probe could not revoke the locked native lease".into());
        }
        transaction.commit().await?;
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
    finish_results(observed, shutdown)?;

    let fenced = snapshot(pool, submitted.job_id).await?;
    if fenced.job_status != "DEAD_LETTERED"
        || fenced.effect_state != "RECONCILE_NEEDED"
        || fenced.provider_effect_id.is_some()
        || !fenced.acceptance_possible
    {
        return Err("expired/reassigned handler mutated provider state after lease loss".into());
    }
    assert_terminal_projection(pool, submitted.delivery_id).await?;
    let after = fixture.counts();
    if after.dispatch_requests - before.dispatch_requests != 1
        || after.accepted_effects - before.accepted_effects != 1
        || after.reconciliation_requests != before.reconciliation_requests
    {
        return Err(format!(
            "lease-fence probe crossed an unexpected provider boundary: before={before:?}, after={after:?}"
        )
        .into());
    }
    Ok(())
}

async fn assert_terminal_projection(pool: &PgPool, delivery_id: Uuid) -> ProbeResult {
    let service = DeliveryService::new(pool.clone());
    let context = OperationContext::new(EXTERNAL_EFFECT_ALLOWANCE)?;
    let owner = OwnerId::new(Uuid::from_u128(OWNER))?;
    let by_id = service
        .get_by_id(&context, owner, delivery_id)
        .await?
        .ok_or("lease-lost delivery missing")?;
    let by_key = service
        .get_by_key(&context, owner, "stale-lease-fence")
        .await?
        .ok_or("lease-lost command missing")?;
    assert_eq!(by_id, by_key);
    assert_eq!(
        by_id.provider.state,
        batter_example_reference_service::delivery::ProviderEffectState::Exhausted
    );
    assert!(by_id.provider.acceptance_possible);
    Ok(())
}

async fn wait_for_effect_fence_wait(pool: &PgPool) -> ProbeResult {
    tokio::time::timeout(EXTERNAL_EFFECT_ALLOWANCE, async {
        loop {
            let waiting: bool = sqlx::query_scalar(
                "SELECT EXISTS (
                    SELECT 1
                      FROM pg_stat_activity
                     WHERE pid <> pg_backend_pid()
                       AND datname = current_database()
                       AND wait_event_type = 'Lock'
                       AND query LIKE '%SELECT id FROM job_queue WHERE id = $1 FOR UPDATE%'
                )",
            )
            .fetch_one(pool)
            .await?;
            if waiting {
                return Ok::<_, sqlx::Error>(());
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .map_err(|_| "provider handler did not reach the live-effect lease fence")??;
    Ok(())
}
