use std::time::Duration;

use batter_sqlx::test_support::{
    CleanupPhase, ConnectionPlan, FixtureError, FixtureSuite, SessionObserver,
};
use sqlx::{Connection, PgConnection, PgPool, postgres::PgPoolOptions};
use tokio::sync::oneshot;

use super::{
    ProbeResult,
    fixture_observation::{BOUND, admin},
    fixture_run,
    fixture_sessions::phase,
};

pub async fn autovacuum_retains_lease() -> ProbeResult {
    let catalog = admin().await?;
    let observer = admin().await?;
    let control = SessionObserver::new(observer.clone(), Duration::from_millis(200))?;
    let (send, receive) = oneshot::channel();
    let (release, gate) = oneshot::channel();
    let run = FixtureSuite::new(super::harness().await?)
        .with_session_observer(control.clone())
        .start(move |scope| {
            Box::pin(async move {
                let plan = ConnectionPlan::new(vec![PgPoolOptions::new().max_connections(1)], 1)?;
                let db = scope.empty(&plan).await?;
                let relation = prepare(&db.pools()[0])
                    .await
                    .map_err(FixtureError::Observe)?;
                send.send((
                    db.database_name().to_owned(),
                    db.pools()[0].connect_options(),
                    relation,
                ))
                .unwrap_or_else(|_| panic!("observer alive"));
                gate.await.unwrap();
                Ok::<_, FixtureError>(())
            })
        });
    let (name, options, relation) = tokio::time::timeout(BOUND, receive).await??;
    let pid = wait_worker(&catalog, &name, relation).await?;
    release.send(()).unwrap();
    phase(&run, CleanupPhase::AwaitingRetry).await;
    let active: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_progress_vacuum WHERE pid = $1 AND datname = $2 AND relid::bigint = $3)")
        .bind(pid).bind(&name).bind(relation).fetch_one(&catalog).await?;
    let retained = fixture_run::present(&catalog, &name).await?;
    // Stop further ordinary autovacuum launches before retrying. This native DDL
    // conflicts with the current worker, whose cancellation is server-owned.
    let mut maintenance = PgConnection::connect_with(&options).await?;
    tokio::time::timeout(
        BOUND,
        sqlx::query("ALTER TABLE vacuum_probe SET (autovacuum_enabled = false)")
            .execute(&mut maintenance),
    )
    .await??;
    maintenance.close().await?;
    super::fixture_sessions::wait_sessions_absent(&catalog, &name).await?;
    control.retry_with(observer.clone());
    let report = tokio::time::timeout(BOUND, run.into_report()).await??;
    let absent = fixture_run::absent(&catalog, &name).await;
    tokio::join!(observer.close(), catalog.close());
    absent?;
    assert!(active && retained && !report.is_ok());
    assert!(report.body.is_ok() && report.drain.is_ok() && report.databases[0].result.is_ok());
    assert!(matches!(
        *report.databases[0].observation_failures[0],
        FixtureError::ObservationTimeout
    ));
    Ok(())
}

async fn prepare(pool: &PgPool) -> Result<i64, sqlx::Error> {
    sqlx::query(
        "CREATE TABLE vacuum_probe (id integer, padding text) WITH (autovacuum_enabled = false)",
    )
    .execute(pool)
    .await?;
    sqlx::query("ALTER TABLE vacuum_probe ALTER COLUMN padding SET STORAGE PLAIN")
        .execute(pool)
        .await?;
    sqlx::query("INSERT INTO vacuum_probe SELECT n, repeat(md5(n::text), 32) FROM generate_series(1, 10000) n").execute(pool).await?;
    sqlx::query("DELETE FROM vacuum_probe WHERE id % 2 = 0")
        .execute(pool)
        .await?;
    sqlx::query("ALTER TABLE vacuum_probe SET (autovacuum_enabled = true, autovacuum_vacuum_threshold = 0, autovacuum_vacuum_scale_factor = 0, autovacuum_vacuum_cost_delay = 100, autovacuum_vacuum_cost_limit = 1, vacuum_truncate = false)").execute(pool).await?;
    sqlx::query("SELECT pg_stat_force_next_flush()")
        .execute(pool)
        .await?;
    sqlx::query_scalar("SELECT 'vacuum_probe'::regclass::oid::bigint")
        .fetch_one(pool)
        .await
}

async fn wait_worker(
    catalog: &PgPool,
    name: &str,
    relation: i64,
) -> Result<i32, Box<dyn std::error::Error + Send + Sync>> {
    Ok(tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            // Follow the intended cost-limited table vacuum while most heap
            // scanning remains, never a launcher visit or an unrelated table.
            let pid = sqlx::query_scalar("SELECT v.pid FROM pg_stat_progress_vacuum v JOIN pg_stat_activity a USING (pid) WHERE v.datname = $1 AND v.relid::bigint = $2 AND a.backend_type = 'autovacuum worker' AND v.heap_blks_total > 0 AND v.heap_blks_scanned < v.heap_blks_total / 2")
                .bind(name).bind(relation).fetch_optional(catalog).await?;
            if let Some(pid) = pid { return Ok::<_, sqlx::Error>(pid); }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }).await??)
}
