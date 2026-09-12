use super::{ProbeResult, startup_process::StartupChild};
use batter_example_reference_service::schema::initialize_schema;
use sqlx::{ConnectOptions, PgPool};
use std::time::Duration;

pub async fn during_database_initialization(pool: PgPool) -> ProbeResult {
    initialize_schema(&pool).await?;
    let mut endpoint = pool.connect_options().to_url_lossy();
    // SQLx adds tuning parameters that the production root deliberately rejects.
    endpoint.set_query(Some("sslmode=disable"));
    let endpoint = endpoint.to_string();
    for (stage, lock, query) in [(
        "postgres.schema",
        "LOCK TABLE _sqlx_migrations IN ACCESS EXCLUSIVE MODE",
        "_sqlx_migrations",
    )] {
        let mut blocker = pool.begin().await?;
        // Identifiers are exclusively the two static fixture tables above.
        sqlx::query(lock).execute(&mut *blocker).await?;
        let child = StartupChild::start(stage, &endpoint)?;
        wait_blocked(&pool, query).await.unwrap_or_else(|_| {
            panic!(
                "child did not reach blocked {stage}: {}",
                child.diagnostics()
            )
        });
        let outcome = tokio::task::spawn_blocking(move || child.terminate()).await?;
        blocker.rollback().await?;
        assert!(outcome.is_ok(), "{stage} child failed: {outcome:?}");
    }
    Ok(())
}

pub async fn during_pool_acquisition() -> ProbeResult {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let endpoint = format!(
        "postgres://fixture:fixture@{}/fixture?sslmode=disable",
        listener.local_addr()?
    );
    let child = StartupChild::start("postgres.acquire", &endpoint)?;
    // Accept proves the initializer reached native acquisition after installing
    // signals. Withhold authentication to keep that real stage pending.
    let (socket, _) = tokio::time::timeout(Duration::from_secs(3), listener.accept()).await??;
    let outcome = tokio::task::spawn_blocking(move || child.terminate()).await?;
    drop(socket);
    outcome.map_err(Into::into)
}

async fn wait_blocked(pool: &PgPool, query: &str) -> ProbeResult {
    tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            let blocked: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM pg_stat_activity WHERE datname = current_database() AND pid <> pg_backend_pid() AND wait_event_type = 'Lock' AND query LIKE $1)")
                .bind(format!("%{query}%")).fetch_one(pool).await?;
            if blocked { return Ok::<_, sqlx::Error>(()); }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await??;
    Ok(())
}
