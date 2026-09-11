use batter_sqlx::test_support::{
    CleanupPhase, ConnectionPlan, FixtureError, FixtureSuite, SessionObserver, template_spec,
};
use sqlx::postgres::PgPoolOptions;

use super::{
    ProbeResult,
    fixture_observation::{BOUND, admin},
    fixture_run,
    fixture_sessions::phase,
};

pub async fn template_recovery() -> ProbeResult {
    let catalog = admin().await?;
    let broken = admin().await?;
    broken.close().await;
    let control = SessionObserver::new(broken, BOUND)?;
    let run = FixtureSuite::new(super::harness().await?)
        .with_session_observer(control.clone())
        .start(|scope| {
            Box::pin(async move {
                let spec = template_spec(&[], "observed-template-recovery-v1");
                let template = scope
                    .template(spec, |_| async {
                        Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
                    })
                    .await?;
                let plan = ConnectionPlan::new(vec![PgPoolOptions::new().max_connections(1)], 0)?;
                scope.from_template(&template, &plan).await?;
                Ok::<_, FixtureError>(template.database_name().to_owned())
            })
        });
    phase(&run, CleanupPhase::AwaitingRetry).await;
    let before = run.cleanup_progress();
    let retained = fixture_run::present(&catalog, &before[0].database_name).await?;
    control.retry_with(catalog.clone());
    let report = tokio::time::timeout(BOUND, run.into_report()).await??;
    let template = report.body.as_ref().unwrap();
    let template_present = fixture_run::present(&catalog, template).await?;
    fixture_run::absent(&catalog, &before[0].database_name).await?;
    catalog.close().await;
    assert!(retained && template_present && !report.is_ok());
    assert!(report.databases[0].result.is_ok() && report.drain.is_ok());
    assert!(std::sync::Arc::ptr_eq(
        &before[0].observation_failures[0],
        &report.databases[0].observation_failures[0]
    ));
    Ok(())
}
