use std::error::Error;

use batter_sqlx::test_support::{FixtureBody, FixtureScope, FixtureSuite, SessionObserver};
use sqlx::{PgPool, postgres::PgPoolOptions};

pub use super::fixture_diagnostics::ProbeError;

pub async fn absent(observer: &PgPool, name: &str) -> super::ProbeResult {
    let exists = present(observer, name).await?;
    if exists {
        return Err("awaited cleanup left a disposable database".into());
    }
    Ok(())
}

pub async fn present(observer: &PgPool, name: &str) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_database WHERE datname = $1)")
        .bind(name)
        .fetch_one(observer)
        .await
}

pub async fn wait_absent(observer: &PgPool, name: &str) -> super::ProbeResult {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while present(observer, name).await? {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        Ok::<_, sqlx::Error>(())
    })
    .await??;
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
    run_with_bound(std::time::Duration::from_secs(30), body).await
}

pub async fn run_with_bound<T, F>(
    bound: std::time::Duration,
    body: F,
) -> Result<T, Box<dyn Error + Send + Sync>>
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
    // Two separate one-slot suite pools: body/catalog diagnostics and session
    // quiescence. Neither borrows capacity from a closing application pool.
    let observer = PgPoolOptions::new()
        .max_connections(1)
        .connect(harness.admin_database_url())
        .await?;
    let body_observer = observer.clone();
    let session_pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(harness.admin_database_url())
        .await?;
    let session_observer =
        SessionObserver::new(session_pool.clone(), std::time::Duration::from_secs(10))?;
    let run = FixtureSuite::new(harness)
        .with_session_observer(session_observer.clone())
        .start(move |scope| {
            Box::pin(async move { body(scope, body_observer).await.map_err(ProbeError::new) })
        });
    let report = super::fixture_completion::ObservedRun::new(
        run,
        session_observer,
        session_pool,
        observer.clone(),
    )
    .finish(bound)
    .await?;
    let observation = async {
        for database in &report.databases {
            absent(&observer, &database.database_name).await?;
        }
        Ok::<_, Box<dyn Error + Send + Sync>>(())
    }
    .await;
    observer.close().await;
    // A pending error owns the unfinished run and both administrative pools. A completed
    // report means cleanup has run before interpreting either result. An observer failure
    // cannot overwrite the complete body/acquisition/database/drain report.
    let result = report.into_result().map_err(ProbeError::report);
    batter_test_support::finish(result, observation.map_err(ProbeError::new))
        .map_err(|error| Box::new(error) as _)
}
