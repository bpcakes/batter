//! A real invariant-recovery UPDATE retains the finalization reserve.
use super::super::super::phased::Invocation;
use super::*;

pub(super) async fn finalization(worker: &DeliveryWorker) -> Result<(), BoxError> {
    let (mut context, payload) = claimed(&worker.pool).await?;
    context.attempt = sqlx::query_scalar(
        "UPDATE job_queue SET attempt = max_attempts WHERE id = $1 RETURNING attempt",
    )
    .bind(context.job_id)
    .fetch_one(&worker.pool)
    .await?;
    // A malformed retained payload enters the existing recovery path without
    // changing lease identity or relying on a concurrent application writer.
    sqlx::query("UPDATE reference_delivery_effects SET provider_payload = '{}'::jsonb WHERE delivery_id = $1")
        .bind(payload.delivery_id).execute(&worker.pool).await?;
    let mut locked = worker.pool.begin().await?;
    sqlx::query(
        "SELECT delivery_id FROM reference_delivery_effects WHERE delivery_id = $1 FOR UPDATE",
    )
    .bind(payload.delivery_id)
    .execute(&mut *locked)
    .await?;
    let handler = PhasedJob::new(
        Invocation::new(worker, &payload),
        PROBE_BUDGET - Duration::from_secs(2),
    )?;
    let invocation = HandlerInvocation::for_context(PROBE_BUDGET, context)?;
    let (run, released) = tokio::join!(invocation.execute(&handler, Value::Null), async {
        // Witness the actual UPDATE, then keep its row lock for longer than
        // the entire two-second work budget. Finalization still has time.
        let observed = tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let waiting: bool = sqlx::query_scalar(
                    "SELECT EXISTS (SELECT 1 FROM pg_stat_activity
                     WHERE datname = current_database() AND wait_event_type = 'Lock'
                       AND query LIKE 'UPDATE reference_delivery_effects e%'
                       AND query LIKE '%retained_state_invariant%')",
                )
                .fetch_one(&worker.pool)
                .await?;
                if waiting {
                    return Ok::<_, sqlx::Error>(());
                }
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
        })
        .await;
        if matches!(observed, Ok(Ok(()))) {
            tokio::time::sleep(Duration::from_secs(3)).await;
        }
        // Release even if the observation failed, then retain that failure.
        locked.rollback().await?;
        observed??;
        Ok::<_, BoxError>(())
    });
    released?;
    assert_eq!(returned_failure(run).code, CODE_MANUAL_RESOLUTION);
    let (state, code): (String, Option<String>) = sqlx::query_as(
        "SELECT state, outcome_code FROM reference_delivery_effects WHERE delivery_id = $1",
    )
    .bind(payload.delivery_id)
    .fetch_one(&worker.pool)
    .await?;
    assert_eq!(state, "MANUAL_RESOLUTION");
    assert_eq!(code.as_deref(), Some("retained_state_invariant"));
    Ok(())
}
