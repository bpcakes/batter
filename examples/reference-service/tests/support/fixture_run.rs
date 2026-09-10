use std::error::Error;

use batter_sqlx::test_support::{FixtureBody, FixtureScope, FixtureSuite};
use sqlx::{PgPool, postgres::PgPoolOptions};

pub use super::fixture_diagnostics::ProbeError;

pub async fn absent(observer: &PgPool, name: &str) -> super::ProbeResult {
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_database WHERE datname = $1)")
            .bind(name)
            .fetch_one(observer)
            .await?;
    if exists {
        return Err("awaited cleanup left a disposable database".into());
    }
    Ok(())
}

pub async fn run<T, F>(body: F) -> Result<T, Box<dyn Error + Send + Sync>>
where
    T: Send + Sync + 'static,
    F: for<'a> FnOnce(
            &'a mut FixtureScope,
            PgPool,
        ) -> FixtureBody<'a, T, Box<dyn Error + Send + Sync>>
        + Send
        + 'static,
{
    let harness = super::harness().await?;
    // One separately bounded suite observer, never borrowed from a closing pool.
    let observer = PgPoolOptions::new()
        .max_connections(1)
        .connect(harness.admin_database_url())
        .await?;
    let body_observer = observer.clone();
    let run = FixtureSuite::new(harness).start(move |scope| {
        Box::pin(async move { body(scope, body_observer).await.map_err(ProbeError::new) })
    });
    let report = run.into_report().await;
    let observation = async {
        if let Ok(report) = &report {
            for database in &report.databases {
                absent(&observer, &database.database_name).await?;
            }
        }
        Ok::<_, Box<dyn Error + Send + Sync>>(())
    }
    .await;
    observer.close().await;
    // All cleanup has run before interpreting either result. An observer failure
    // cannot overwrite the complete body/acquisition/database/drain report.
    let result = report
        .map_err(|error| ProbeError::new(Box::new(error)))
        .and_then(|report| report.into_result().map_err(ProbeError::report));
    batter_test_support::finish(result, observation.map_err(ProbeError::new))
        .map_err(|error| Box::new(error) as _)
}
