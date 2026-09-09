use postgres_test_harness::PostgresHarness;
use sqlx::PgPool;

use super::{ProbeResult, harness, pool};

pub async fn probe() -> ProbeResult {
    let harness = harness().await?;
    assert!(harness.is_external());
    assert_eq!(harness.connection_limits().connections_per_database(), 8);
    let admin = pool(harness.admin_database_url()).await?;
    let body: ProbeResult = async {
        for mode in 0..4 {
            one_lease(&harness, &admin, mode).await?;
        }
        Ok(())
    }
    .await;
    admin.close().await;
    let shutdown = harness.shutdown().await;
    let result = batter_test_support::finish(body, shutdown);
    assert!(
        result.is_ok(),
        "lease body/shutdown failed; both causes retained"
    );
    Ok(())
}

async fn one_lease(harness: &PostgresHarness, admin: &PgPool, mode: u8) -> ProbeResult {
    let lease = harness.empty_database().await?;
    let name = lease.database_name().to_owned();
    let application = pool(lease.database_url()).await;
    let body = match application {
        Ok(application) => {
            let result = observe_application(&application, admin, &name).await;
            application.close().await;
            result
        }
        Err(error) => Err(error),
    };
    let cleanup = match mode {
        0 => lease.cleanup().await,
        1 => lease.defer_cleanup().await,
        2 => {
            drop(lease);
            Ok(())
        }
        _ => {
            drop(lease.cleanup());
            Ok(())
        }
    };
    let drained = harness.drain_deferred_cleanup().await;
    let cleanup = batter_test_support::finish(cleanup, drained);
    let (value, present) = batter_test_support::finish(body, cleanup)?;
    assert_eq!(value, 1);
    assert!(present);
    let present: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_database WHERE datname = $1)")
            .bind(&name)
            .fetch_one(admin)
            .await?;
    assert!(!present, "drained lease must be absent");
    Ok(())
}

async fn observe_application(
    application: &PgPool,
    admin: &PgPool,
    name: &str,
) -> Result<(i32, bool), sqlx::Error> {
    let value: i32 = sqlx::query_scalar("SELECT 1")
        .fetch_one(application)
        .await?;
    let present: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_database WHERE datname = $1)")
            .bind(name)
            .fetch_one(admin)
            .await?;
    Ok((value, present))
}
