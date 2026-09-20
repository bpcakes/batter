use super::*;

/// Kill the worker after the application outcome commits but while native
/// completion is blocked. Redelivery must retain the provider's lower bound.
pub(super) async fn probe(
    pool: &PgPool,
    fixture: &ProviderFixture,
    endpoint: &str,
    provider_url: &str,
) -> ProbeResult {
    fixture.set_dispatch(DispatchBehavior::RateLimited {
        retry_after_ms: 86_400_000,
    });
    let before = fixture.counts();
    let submitted = submit(pool, 0x41e, "provider-retry-interruption", 16).await?;
    sqlx::raw_sql(
        "CREATE FUNCTION reference_pause_provider_completion() RETURNS trigger LANGUAGE plpgsql AS $$
         BEGIN
           IF OLD.status::text = 'LEASED' AND NEW.status::text = 'PENDING'
              AND NEW.last_error_code = 'delivery.provider_undispatched' THEN
             PERFORM pg_advisory_xact_lock(821234);
           END IF;
           RETURN NEW;
         END $$;
         CREATE TRIGGER reference_pause_provider_completion BEFORE UPDATE ON job_queue
         FOR EACH ROW EXECUTE FUNCTION reference_pause_provider_completion();",
    ).execute(pool).await?;
    let mut blocker = pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(821234)")
        .execute(&mut *blocker)
        .await?;
    let first =
        ExecutableChild::start_provider_worker(endpoint, provider_url, "retry-interrupted")?;
    let observed = observe_blocked_completion(pool, submitted.job_id).await;
    // Always settle the started process and the explicit database lock first.
    let crashed = crash(first, endpoint.to_owned(), provider_url.to_owned()).await;
    let unlocked = blocker
        .rollback()
        .await
        .map_err(|error| Box::new(error) as BoxError);
    let removed = sqlx::raw_sql(
        "DROP TRIGGER reference_pause_provider_completion ON job_queue;
         DROP FUNCTION reference_pause_provider_completion();",
    )
    .execute(pool)
    .await
    .map(|_| ())
    .map_err(|error| Box::new(error) as BoxError);
    finish_results(
        finish_results(observed, crashed),
        finish_results(unlocked, removed),
    )?;

    fixture.set_dispatch(DispatchBehavior::Accept);
    let second =
        ExecutableChild::start_provider_worker(endpoint, provider_url, "retry-redelivery")?;
    let observed = async {
        let retained =
            wait_for_snapshot(pool, submitted.job_id, EXTERNAL_EFFECT_ALLOWANCE, |state| {
                state.attempt == 2
                    && state.job_status == "PENDING"
                    && state.last_error_code.as_deref() == Some("delivery.provider_retry_deferred")
            })
            .await?;
        if retained.effect_state != "RETRYABLE_UNDISPATCHED" || retained.acceptance_possible {
            return Err::<_, BoxError>("retry recovery changed known non-dispatch truth".into());
        }
        let timing_retained: bool = sqlx::query_scalar(
            "SELECT e.retry_not_before > clock_timestamp() + interval '23 hours'
                AND q.next_run_at >= e.retry_not_before
               FROM reference_delivery_effects e JOIN reference_deliveries d ON d.id = e.delivery_id
               JOIN job_queue q ON q.id = d.job_id WHERE q.id = $1",
        )
        .bind(submitted.job_id)
        .fetch_one(pool)
        .await?;
        if !timing_retained
            || fixture.counts().dispatch_requests != before.dispatch_requests + 1
            || fixture.counts().accepted_effects != before.accepted_effects
        {
            return Err(
                "interrupted native completion bypassed durable provider eligibility".into(),
            );
        }
        Ok(())
    }
    .await;
    let stopped = stop(
        second,
        Signal::Term,
        endpoint.to_owned(),
        provider_url.to_owned(),
    )
    .await;
    finish_results(observed, stopped)
}

async fn observe_blocked_completion(pool: &PgPool, job_id: Uuid) -> ProbeResult {
    tokio::time::timeout(EXTERNAL_EFFECT_ALLOWANCE, async {
        loop {
            let waiting: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM pg_locks l JOIN pg_stat_activity a ON a.pid = l.pid
                    WHERE a.datname = current_database() AND l.locktype = 'advisory'
                      AND NOT l.granted AND l.objid = 821234)",
            )
            .fetch_one(pool)
            .await?;
            if waiting {
                break;
            }
            tokio::task::yield_now().await;
        }
        let retained = snapshot(pool, job_id).await?;
        if retained.job_status != "LEASED" || retained.effect_state != "RETRYABLE_UNDISPATCHED" {
            return Err::<_, BoxError>("did not witness the application/native commit gap".into());
        }
        Ok(())
    })
    .await
    .map_err(|_| "native provider completion never reached the interruption barrier")?
}
