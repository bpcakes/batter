use super::*;

pub(super) async fn ordering(worker: &DeliveryWorker) -> Result<(), BoxError> {
    replacement_first(worker, false).await?;
    replacement_first(worker, true).await?;
    confirmation_first(worker).await
}

async fn prepared(worker: &DeliveryWorker) -> Result<(JobContext, DeliveryJobPayload), BoxError> {
    let (context, payload) = claimed(&worker.pool).await?;
    let services = Services(Instant::now() + Duration::from_secs(10));
    assert!(
        worker
            .mark_dispatch_possible(
                JobExecution::new(&context, &services),
                &payload,
                ProviderEffectState::AwaitingAttempt,
            )
            .await
            .unwrap()
    );
    Ok((context, payload))
}

/// Return the actual blocked backend, not a timing-only inference.
async fn waiter(pool: &PgPool, blocker: i32) -> Result<i32, BoxError> {
    Ok(tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let waiting: Option<i32> = sqlx::query_scalar(
                "SELECT pid FROM pg_stat_activity WHERE datname = current_database()
                    AND pid <> pg_backend_pid() AND $1 = ANY(pg_blocking_pids(pid))",
            )
            .bind(blocker)
            .fetch_optional(pool)
            .await?;
            if let Some(pid) = waiting {
                return Ok::<_, sqlx::Error>(pid);
            }
            tokio::task::yield_now().await;
        }
    })
    .await??)
}

async fn replacement_first(worker: &DeliveryWorker, expire: bool) -> Result<(), BoxError> {
    let (context, payload) = prepared(worker).await?;
    if expire {
        sqlx::query("UPDATE job_queue SET lease_expires_at = clock_timestamp() + interval '2 seconds' WHERE id = $1")
            .bind(context.job_id).execute(&worker.pool).await?;
    }
    let services = Services(Instant::now() + Duration::from_secs(10));
    let mut replacement = worker.pool.begin().await?;
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *replacement)
        .await?;
    sqlx::query("UPDATE reference_records SET generation = 2 WHERE id = $1")
        .bind(payload.record_id)
        .execute(&mut *replacement)
        .await?;
    let confirmation = tokio::time::timeout(
        Duration::from_secs(8),
        worker.confirm(
            JobExecution::new(&context, &services),
            &payload,
            "fixture-accepted".into(),
        ),
    );
    let release = async {
        waiter(&worker.pool, pid).await?;
        // The record wait must not starve native lease maintenance.
        let mut heartbeat = worker.pool.begin().await?;
        sqlx::query("SELECT id FROM job_queue WHERE id = $1 FOR UPDATE NOWAIT")
            .bind(context.job_id)
            .execute(&mut *heartbeat)
            .await?;
        heartbeat.rollback().await?;
        if expire {
            tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    let expired: bool = sqlx::query_scalar(
                        "SELECT lease_expires_at <= clock_timestamp() FROM job_queue WHERE id = $1",
                    )
                    .bind(context.job_id)
                    .fetch_one(&worker.pool)
                    .await?;
                    if expired {
                        return Ok::<_, sqlx::Error>(());
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await??;
        }
        replacement.commit().await?;
        Ok::<_, BoxError>(())
    };
    let (confirmed, released) = tokio::join!(confirmation, release);
    released?;
    let failure = confirmed?.expect_err("a replaced generation cannot confirm");
    let (state, code, effect): (String, String, Option<String>) = sqlx::query_as(
        "SELECT state, outcome_code, provider_effect_id FROM reference_delivery_effects WHERE delivery_id = $1",
    ).bind(payload.delivery_id).fetch_one(&worker.pool).await?;
    if expire {
        assert_eq!(failure.code, CODE_LEASE_LOST);
        assert_eq!(state, "RECONCILE_NEEDED");
        assert_eq!(code, "dispatch_possible");
        assert!(effect.is_none());
    } else {
        assert_eq!(failure.code, CODE_MANUAL_RESOLUTION);
        assert_eq!(state, "MANUAL_RESOLUTION");
        assert_eq!(code, "target_changed_after_dispatch");
        assert_eq!(effect.as_deref(), Some("fixture-accepted"));
    }
    Ok(())
}

async fn confirmation_first(worker: &DeliveryWorker) -> Result<(), BoxError> {
    let (context, payload) = prepared(worker).await?;
    let services = Services(Instant::now() + Duration::from_secs(10));
    let mut effect_blocker = worker.pool.begin().await?;
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *effect_blocker)
        .await?;
    sqlx::query(
        "SELECT delivery_id FROM reference_delivery_effects WHERE delivery_id = $1 FOR UPDATE",
    )
    .bind(payload.delivery_id)
    .execute(&mut *effect_blocker)
    .await?;
    let confirmation = tokio::time::timeout(
        Duration::from_secs(8),
        worker.confirm(
            JobExecution::new(&context, &services),
            &payload,
            "fixture-accepted".into(),
        ),
    );
    let replace = async {
        let confirming_pid = waiter(&worker.pool, pid).await?;
        let replacement = tokio::time::timeout(Duration::from_secs(5), async {
            let mut transaction = worker.pool.begin().await?;
            sqlx::query("UPDATE reference_records SET generation = 2 WHERE id = $1")
                .bind(payload.record_id)
                .execute(&mut *transaction)
                .await?;
            // Once replacement owns the record, confirmation must be committed,
            // not merely finished evaluating its statement-snapshot predicate.
            let state: String = sqlx::query_scalar(
                "SELECT state FROM reference_delivery_effects WHERE delivery_id = $1",
            )
            .bind(payload.delivery_id)
            .fetch_one(&mut *transaction)
            .await?;
            assert_eq!(state, "CONFIRMED");
            transaction.commit().await?;
            Ok::<_, BoxError>(())
        });
        let release = async {
            waiter(&worker.pool, confirming_pid).await?;
            effect_blocker.commit().await?;
            Ok::<_, BoxError>(())
        };
        let (replaced, released) = tokio::join!(replacement, release);
        released?;
        replaced??;
        Ok::<_, BoxError>(())
    };
    let (confirmed, replaced) = tokio::join!(confirmation, replace);
    replaced?;
    assert!(confirmed?.is_ok());
    let (state, generation): (String, i64) = sqlx::query_as(
        "SELECT e.state, r.generation FROM reference_delivery_effects e
            JOIN reference_records r ON r.id = e.record_id WHERE e.delivery_id = $1",
    )
    .bind(payload.delivery_id)
    .fetch_one(&worker.pool)
    .await?;
    assert_eq!((state.as_str(), generation), ("CONFIRMED", 2));
    Ok(())
}
