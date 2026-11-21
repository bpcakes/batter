use crate::support::configuration::configuration_report::PoolClosure;
use crate::support::configuration::configuration_report::ReadinessObservation;

#[path = "configuration_report.rs"]
mod configuration_report;
use super::{ProbeResult, fixture_run};
use batter::sqlx::test_support::ConnectionPlan;
use batter::{
    cleanup::CleanupBudget,
    lifecycle::{ProcessCapacity, Readiness, ShutdownBudget, Supervisor},
    settings::SettingsSource,
    startup::{Startup, StartupError},
};
use batter_example_reference_service::config::PoolSettings;
use sqlx::postgres::PgConnectOptions;
use std::time::Duration;

pub async fn one_slot_pool() -> ProbeResult {
    fixture_run::run(|scope, _observer| {
        Box::pin(async move {
            let settings = PoolSettings::from_source(&SettingsSource::from_pairs([
                ("BATTER_POOL_MAX_CONNECTIONS".into(), "1".into()),
                ("BATTER_POOL_MIN_CONNECTIONS".into(), "0".into()),
                ("BATTER_POOL_ACQUIRE_TIMEOUT_MS".into(), "100".into()),
            ])?)?;
            let plan = ConnectionPlan::new(vec![settings.pool_options()], 0)?;
            let fixture = scope.empty(&plan).await?;
            let pool = &fixture.pools()[0];
            let held = pool.acquire().await?;
            let mut second = Box::pin(pool.acquire());
            let pending = std::future::poll_fn(|cx| {
                use std::future::Future;
                std::task::Poll::Ready(second.as_mut().poll(cx).is_pending())
            })
            .await;
            let result = second.await;
            drop(held);
            let mut reused = pool.acquire().await?;
            let value: i32 = sqlx::query_scalar("SELECT 73")
                .fetch_one(&mut *reused)
                .await?;
            drop(reused);
            if !pending || !matches!(result, Err(sqlx::Error::PoolTimedOut)) || value != 73 {
                return Err("configured one-slot acquisition contract failed".into());
            }
            Ok(())
        })
    })
    .await
}

pub async fn failed_startup_closes_pool_before_lease() -> ProbeResult {
    let harness = super::harness().await?;
    let lease = harness.empty_database().await?;
    let name = lease.database_name().to_owned();
    // Options come from the harness's own endpoint parse; a refusal returns
    // as the body's failure, so the lease cleanup below is still awaited.
    let body = match batter::sqlx::test_support::harness_connect_options(&harness, &lease) {
        Ok(options) => observe_failed_startup(options).await,
        Err(error) => Err(error.into()),
    };
    // Native lease remains owned until awaited startup/pool closure is inspected.
    let cleanup = lease.cleanup().await;
    let drain = harness.drain_deferred_cleanup().await;
    let observer = super::pool(batter::sqlx::test_support::harness_admin_connect_options(
        &harness,
    ))
    .await;
    let absence = match observer {
        Ok(observer) => {
            let result = fixture_run::absent(&observer, &name).await;
            observer.close().await;
            result
        }
        Err(error) => Err(error.into()),
    };
    let shutdown = harness.shutdown().await;
    let cleanup = batter::test_support::finish(cleanup, drain);
    let cleanup = batter::test_support::finish(cleanup, shutdown);
    let checked = batter::test_support::finish(
        body.map_err(super::fixture_diagnostics::ProbeError::new),
        absence.map_err(super::fixture_diagnostics::ProbeError::new),
    );
    batter::test_support::finish(checked, cleanup)?;
    Ok(())
}

async fn observe_failed_startup(connect_options: PgConnectOptions) -> ProbeResult {
    let pool_options = PoolSettings::new(1, 0, Duration::from_secs(3))?.pool_options();
    let cleanup = CleanupBudget::new(
        Duration::from_secs(2),
        Duration::from_secs(2),
        Duration::from_secs(1),
    )?;
    let supervisor = Supervisor::with_process_capacity(
        ShutdownBudget::new(
            Duration::from_secs(1),
            Duration::from_secs(1),
            Duration::from_secs(1),
            cleanup,
        )?,
        ProcessCapacity::new(32)?,
    );
    let handle = supervisor.handle();
    let (pool_tx, pool_rx) = tokio::sync::oneshot::channel();
    let starting = Startup::new(
        supervisor,
        batter::operation::OperationOwner::new(Duration::from_secs(10))?.into_context(),
        cleanup,
        move |scope| {
            Box::pin(async move {
                scope.stage("pool.acquire").map_err(|e| {
                    batter::settings::SettingsError::new("startup", "invalid stage").with_cause(e)
                })?;
                let slot = scope
                    .supervisor()
                    .reserve_cleanup("pool.close")
                    .map_err(|e| {
                        batter::settings::SettingsError::new("startup", "invalid reservation")
                            .with_cause(e)
                    })?;
                let pool = pool_options
                    .connect_with(connect_options)
                    .await
                    .map_err(|e| {
                        batter::settings::SettingsError::new("pool", "acquisition failed")
                            .with_cause(e)
                    })?;
                let closing = pool.clone();
                slot.register(move || async move {
                    closing.close().await;
                    Ok(())
                });
                let _ = pool_tx.send(pool);
                scope.stage("later.initialize").map_err(|e| {
                    batter::settings::SettingsError::new("startup", "invalid stage").with_cause(e)
                })?;
                Err::<(), _>(
                    batter::settings::SettingsError::new("later", "initialization failed")
                        .with_cause(std::io::Error::other("private-startup-marker")),
                )
            })
        },
    )
    .start();
    let result = starting.finish().await;
    let acquired = pool_rx.await;
    let closed_before_lease = acquired.as_ref().is_ok_and(|pool| pool.is_closed());
    // Preserve the observation before emergency cleanup. A failed test must
    // still close a delivered pool before allowing fixture lease disposal.
    if let Ok(pool) = &acquired {
        pool.close().await;
    }
    let no_ready = handle.status().readiness() != Readiness::Ready;
    let report = match result {
        Err(StartupError::Failed(report)) => report,
        Err(error) => return Err(Box::new(error) as batter::BoxError),
        Ok(running) => {
            let shutdown = running.shutdown().await;
            batter::lifecycle::check_shutdown(shutdown)?;
            return Err("startup unexpectedly reached running state".into());
        }
    };
    configuration_report::check(
        report,
        acquired.map(|_| ()),
        if closed_before_lease {
            PoolClosure::Closed
        } else {
            PoolClosure::Open
        },
        if no_ready {
            ReadinessObservation::Unready
        } else {
            ReadinessObservation::Ready
        },
    )?;
    Ok::<_, batter::BoxError>(())
}
