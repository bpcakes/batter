#[path = "configuration_report.rs"]
mod configuration_report;
use super::{ProbeResult, fixture_run};
use batter::{
    cleanup::CleanupBudget,
    lifecycle::{Readiness, ShutdownBudget},
    operation::OperationContext,
    settings::SettingsSource,
    startup::{Startup, StartupError},
};
use batter_example_reference_service::config::{ConfigMode, PoolSettings, RootSettings};
use batter_sqlx::test_support::ConnectionPlan;
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
    let body = observe_failed_startup(lease.database_url()).await;
    // Native lease remains owned until awaited startup/pool closure is inspected.
    let cleanup = lease.cleanup().await;
    let drain = harness.drain_deferred_cleanup().await;
    let observer = super::pool(harness.admin_database_url()).await;
    let absence = match observer {
        Ok(observer) => {
            let result = fixture_run::absent(&observer, &name).await;
            observer.close().await;
            result
        }
        Err(error) => Err(error.into()),
    };
    let shutdown = harness.shutdown().await;
    let cleanup = batter_test_support::finish(cleanup, drain);
    let cleanup = batter_test_support::finish(cleanup, shutdown);
    let checked = batter_test_support::finish(
        body.map_err(super::fixture_diagnostics::ProbeError::new),
        absence.map_err(super::fixture_diagnostics::ProbeError::new),
    );
    batter_test_support::finish(checked, cleanup)?;
    Ok(())
}

fn startup_settings(database_url: &str) -> Result<RootSettings, batter::BoxError> {
    // The fixture supplies an explicit endpoint; add its known local TLS
    // policy only when absent, without altering another requested mode.
    let mut url = url::Url::parse(database_url)?;
    if !url.query_pairs().any(|(key, _)| key == "sslmode") {
        url.query_pairs_mut().append_pair("sslmode", "disable");
    }
    Ok(RootSettings::from_sources(
        ConfigMode::Setup,
        None,
        SettingsSource::default(),
        SettingsSource::from_pairs([
            ("DATABASE_URL".into(), url.as_str().into()),
            ("BATTER_POOL_MAX_CONNECTIONS".into(), "1".into()),
        ])?,
    )?)
}

async fn observe_failed_startup(database_url: &str) -> ProbeResult {
    let settings = startup_settings(database_url)?;
    let cleanup = CleanupBudget::new(
        Duration::from_secs(2),
        Duration::from_secs(2),
        Duration::from_secs(1),
    )?;
    let supervisor = settings.supervisor(ShutdownBudget::new(
        Duration::from_secs(1),
        Duration::from_secs(1),
        Duration::from_secs(1),
        cleanup,
    )?)?;
    let handle = supervisor.handle();
    let (pool_tx, pool_rx) = tokio::sync::oneshot::channel();
    let mut starting = Startup::new(
        supervisor,
        OperationContext::new(Duration::from_secs(10))?,
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
                let options = settings.connect_options_from_process()?;
                let pool = settings
                    .pool_options()
                    .connect_with(options)
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
    let result = starting.wait().await;
    let acquired = pool_rx.await;
    let closed_before_lease = acquired.as_ref().is_ok_and(|pool| pool.is_closed());
    // Preserve the observation before emergency cleanup. A failed test must
    // still close a delivered pool before allowing fixture lease disposal.
    if let Ok(pool) = &acquired {
        pool.close().await;
    }
    let no_ready = handle.readiness() != Readiness::Ready;
    let report = match result {
        Err(StartupError::Failed(report)) => report,
        Err(error) => return Err(Box::new(error) as batter::BoxError),
        Ok(running) => {
            let shutdown = running.shutdown().await;
            batter::lifecycle::check_shutdown(shutdown)?;
            return Err("startup unexpectedly reached running state".into());
        }
    };
    configuration_report::check(report, acquired.map(|_| ()), closed_before_lease, no_ready)?;
    Ok::<_, batter::BoxError>(())
}
