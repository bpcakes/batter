use super::super::provider::LookupBehavior;
use super::*;
use batter_example_reference_service::delivery::{QueryError, StorageError};
use serde_json::Value;

/// Corrupt each identity input independently before any worker can load it.
pub(super) async fn identity(
    pool: &PgPool,
    fixture: &ProviderFixture,
    endpoint: &str,
    provider_url: &str,
) -> ProbeResult {
    fixture.set_dispatch(DispatchBehavior::Accept);
    fixture.set_lookup(LookupBehavior::Stored);
    let before = fixture.counts();
    let service = DeliveryService::new(pool.clone());
    let owner = OwnerId::new(Uuid::from_u128(OWNER))?;
    let mut commands = Vec::new();
    for (record, key, corrupt_payload) in [
        (0x420, "retained-payload-mismatch", true),
        (0x421, "retained-key-mismatch", false),
    ] {
        let submitted = submit(pool, record, key, 20).await?;
        let changed = if corrupt_payload {
            sqlx::query("UPDATE reference_delivery_effects SET provider_payload = jsonb_set(provider_payload, '{payload,value}', '999'::jsonb) WHERE delivery_id = $1")
                .bind(submitted.delivery_id).execute(pool).await?
        } else {
            sqlx::query("UPDATE reference_delivery_effects SET provider_key = 'fixture:wrong-key' WHERE delivery_id = $1")
                .bind(submitted.delivery_id).execute(pool).await?
        };
        if changed.rows_affected() != 1 {
            return Err("identity corruption did not update exactly one row".into());
        }
        let context = OperationContext::new(Duration::from_secs(5))?;
        for result in [
            service
                .get_by_id(&context, owner, submitted.delivery_id)
                .await,
            service.get_by_key(&context, owner, key).await,
        ] {
            if !matches!(result, Err(QueryError::Storage(StorageError::Invariant(_)))) {
                return Err("owner-scoped read did not reject retained identity".into());
            }
        }
        commands.push(submitted);
    }
    let worker =
        ExecutableChild::start_provider_worker(endpoint, provider_url, "retained-identity")?;
    let observed = async {
        for command in commands {
            let retained = wait_for_snapshot(pool, command.job_id, Duration::from_secs(5), |row| {
                row.job_status == "DEAD_LETTERED" && row.effect_state == "MANUAL_RESOLUTION"
            })
            .await?;
            // This code distinguishes load rejection from the later request
            // constructor rejecting an invalid provider key.
            if retained.attempt != 1
                || retained.outcome_code.as_deref() != Some("retained_state_invariant")
                || retained.last_error_code.as_deref()
                    != Some("delivery.manual_resolution_required")
                || retained.provider_effect_id.is_some()
            {
                return Err::<_, BoxError>(
                    "worker did not reject the retained identity at load".into(),
                );
            }
        }
        if fixture.counts() != before {
            return Err("invalid retained identity reached the provider".into());
        }
        Ok(())
    }
    .await;
    let stopped = stop(
        worker,
        Signal::Term,
        endpoint.to_owned(),
        provider_url.to_owned(),
    )
    .await;
    finish_results(observed, stopped)
}

/// A dispatch lower bound never delays the keyed GET that resolves uncertainty.
pub(super) async fn reconciliation(
    pool: &PgPool,
    fixture: &ProviderFixture,
    endpoint: &str,
    provider_url: &str,
) -> ProbeResult {
    for (record, key, accepted) in [
        (0x422, "future-delay-absent", false),
        (0x423, "future-delay-accepted", true),
    ] {
        fixture.set_dispatch(DispatchBehavior::Accept);
        fixture.set_lookup(if accepted {
            LookupBehavior::Stored
        } else {
            LookupBehavior::Absent
        });
        let submitted = submit(pool, record, key, 21).await?;
        if accepted {
            seed_acceptance(pool, provider_url, submitted.delivery_id).await?;
        }
        let before = fixture.counts();
        let original: Value = sqlx::query_scalar(
            "UPDATE reference_delivery_effects e SET state = 'RECONCILE_NEEDED',
                outcome_code = 'dispatch_possible', acceptance_possible = true,
                dispatch_possible_at = clock_timestamp(),
                resolve_before = clock_timestamp() + interval '1 hour',
                retry_not_before = clock_timestamp() + interval '24 hours'
              WHERE delivery_id = $1 RETURNING to_jsonb(e)",
        )
        .bind(submitted.delivery_id)
        .fetch_one(pool)
        .await?;
        let worker = ExecutableChild::start_provider_worker(endpoint, provider_url, key)?;
        let observed = async {
            let retained =
                wait_for_snapshot(pool, submitted.job_id, Duration::from_secs(5), |row| {
                    if accepted {
                        row.job_status == "SUCCEEDED" && row.effect_state == "CONFIRMED"
                    } else {
                        row.job_status == "PENDING"
                            && row.attempt == 1
                            && row.last_error_code.as_deref()
                                == Some("delivery.provider_retry_deferred")
                    }
                })
                .await?;
            let after = fixture.counts();
            if after.reconciliation_requests != before.reconciliation_requests + 1
                || after.dispatch_requests != before.dispatch_requests
                || after.accepted_effects != before.accepted_effects
            {
                return Err::<_, BoxError>("future delay blocked GET or permitted POST".into());
            }
            if accepted {
                if retained.acceptance_possible
                    || retained.provider_effect_id.as_deref()
                        != Some(&format!("provider:{}", submitted.delivery_id))
                {
                    return Err("accepted lookup lost provider identity".into());
                }
            } else {
                let unchanged: Value = sqlx::query_scalar(
                    "SELECT to_jsonb(e) FROM reference_delivery_effects e WHERE delivery_id = $1",
                )
                .bind(submitted.delivery_id)
                .fetch_one(pool)
                .await?;
                if unchanged != original {
                    return Err("absence changed retained uncertainty or timing".into());
                }
                let deferred: bool = sqlx::query_scalar(
                    "SELECT q.next_run_at >= e.retry_not_before
                       FROM job_queue q JOIN reference_deliveries d ON d.job_id = q.id
                       JOIN reference_delivery_effects e ON e.delivery_id = d.id WHERE q.id = $1",
                )
                .bind(submitted.job_id)
                .fetch_one(pool)
                .await?;
                if !deferred {
                    return Err("native retry preceded retained dispatch eligibility".into());
                }
            }
            Ok(())
        }
        .await;
        let stopped = stop(
            worker,
            Signal::Term,
            endpoint.to_owned(),
            provider_url.to_owned(),
        )
        .await;
        finish_results(observed, stopped)?;
    }
    Ok(())
}

async fn seed_acceptance(pool: &PgPool, provider_url: &str, delivery: Uuid) -> ProbeResult {
    let (key, payload): (String, Value) = sqlx::query_as(
        "SELECT provider_key, provider_payload FROM reference_delivery_effects WHERE delivery_id = $1",
    ).bind(delivery).fetch_one(pool).await?;
    let response: Value = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(5))
        .build()?
        .post(format!("{provider_url}effects"))
        .bearer_auth(super::super::provider::TOKEN)
        .header("Idempotency-Key", key)
        .json(&payload)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    if response["provider_effect_id"] != format!("provider:{delivery}") {
        return Err("provider acceptance setup returned an unexpected identity".into());
    }
    Ok(())
}
