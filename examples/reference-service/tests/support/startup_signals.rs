use super::{
    ProbeResult,
    startup_process::{self, ExecutableChild, Signal, StartupChild},
};
use batter_example_reference_service::schema::initialize_schema;
use sqlx::PgPool;
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};

const PRIVATE: &str = "fixture-token";
const SIGNAL_ACKNOWLEDGEMENT_BOUND: Duration = Duration::from_secs(2);

/// Actual `runtime::run` blocked on schema SQL behind an independently observed lock.
pub async fn schema(pool: PgPool, signal: Signal) -> ProbeResult {
    initialize_schema(&pool).await?;
    let endpoint = startup_process::endpoint(&pool);
    let mut blocker = pool.begin().await?;
    let blocker_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *blocker)
        .await?;
    sqlx::query("LOCK TABLE _sqlx_migrations IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *blocker)
        .await?;
    let child = StartupChild::start("postgres.schema", &endpoint)?;
    if wait_blocked(&pool, "_sqlx_migrations", blocker_pid)
        .await
        .is_err()
    {
        // Release the lock after explicitly terminating and reaping the child;
        // its retained stream diagnostics are structurally redacted.
        let diagnostics = child.abort_diagnostics(&[&endpoint, PRIVATE]);
        blocker.rollback().await?;
        return Err(format!("child did not reach blocked postgres.schema: {diagnostics}").into());
    }
    let signal_endpoint = endpoint.clone();
    let requested = tokio::task::spawn_blocking(move || {
        child.request_stop(signal, &[&signal_endpoint, PRIVATE])
    })
    .await;
    let child = match requested {
        Ok(Ok(child)) => child,
        Ok(Err(error)) => {
            blocker.rollback().await?;
            return Err(error.into());
        }
        Err(error) => {
            blocker.rollback().await?;
            return Err(error.into());
        }
    };
    let observed = tokio::task::spawn_blocking(move || {
        let mut child = child;
        let result = child.wait_event(
            startup_process::SIGNAL_OBSERVED,
            SIGNAL_ACKNOWLEDGEMENT_BOUND,
        );
        (child, result)
    })
    .await;
    // Tokio broadcasts the notification to every listener before either receiver
    // can emit the acknowledgement. Release the query after that event but before
    // child cleanup: retaining the lock until reap would make a dropped SQLx
    // checkout's return-to-pool path and pool close cyclic.
    blocker.rollback().await?;
    let (child, observation) = observed?;
    if let Err(error) = observation {
        let diagnostic_endpoint = endpoint.clone();
        let diagnostics = tokio::task::spawn_blocking(move || {
            child.abort_diagnostics(&[&diagnostic_endpoint, PRIVATE])
        })
        .await?;
        return Err(format!("signal acknowledgement failed: {error}; {diagnostics}").into());
    }
    let outcome = tokio::task::spawn_blocking(move || {
        child.finish(&[startup_process::SIGNAL_OBSERVED], &[&endpoint, PRIVATE])
    })
    .await?;
    outcome.map_err(Into::into)
}

/// Actual `runtime::run` held in native acquisition by a withheld handshake.
pub async fn acquisition(signal: Signal) -> ProbeResult {
    let (listener, endpoint) = held_endpoint().await?;
    let child = StartupChild::start("postgres.acquire", &endpoint)?;
    // Accept proves the initializer reached native acquisition after start
    // installed signals. Withhold authentication to keep that real stage pending.
    let socket = match accept(&listener).await {
        Ok(socket) => socket,
        Err(_) => {
            let diagnostics = child.abort_diagnostics(&[&endpoint, PRIVATE]);
            return Err(format!("child did not begin native acquisition: {diagnostics}").into());
        }
    };
    let outcome =
        tokio::task::spawn_blocking(move || child.stop(signal, &[], &[&endpoint, PRIVATE])).await?;
    drop(socket);
    outcome.map_err(Into::into)
}

/// A signal-specific fixture plus the production executable at held acquisition.
pub async fn executable(signal: Signal) -> ProbeResult {
    executable_process(
        signal,
        ExecutableChild::start_signal_fixture,
        "signal fixture",
    )
    .await?;
    executable_process(
        signal,
        ExecutableChild::start_production,
        "production executable",
    )
    .await
}

async fn executable_process(
    signal: Signal,
    start: fn(&str) -> std::io::Result<ExecutableChild>,
    label: &str,
) -> ProbeResult {
    let (listener, endpoint) = held_endpoint().await?;
    let child = start(&endpoint)?;
    let socket = match accept(&listener).await {
        Ok(socket) => socket,
        Err(_) => {
            let diagnostics = child.abort_diagnostics(&[&endpoint, PRIVATE]);
            return Err(format!("{label} did not begin native acquisition: {diagnostics}").into());
        }
    };
    let outcome =
        tokio::task::spawn_blocking(move || child.stop(signal, &[&endpoint, PRIVATE])).await?;
    drop(socket);
    outcome.map_err(Into::into)
}

async fn held_endpoint() -> Result<(TcpListener, String), batter::BoxError> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let endpoint = format!(
        "postgres://fixture:fixture@{}/fixture?sslmode=disable",
        listener.local_addr()?
    );
    Ok((listener, endpoint))
}

async fn accept(listener: &TcpListener) -> Result<TcpStream, batter::BoxError> {
    let (socket, _) = tokio::time::timeout(Duration::from_secs(3), listener.accept()).await??;
    Ok(socket)
}

async fn wait_blocked(pool: &PgPool, query: &str, blocker: i32) -> ProbeResult {
    tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            let blocked: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM pg_stat_activity WHERE datname = current_database() AND pid <> pg_backend_pid() AND wait_event_type = 'Lock' AND query LIKE $1 AND $2 = ANY(pg_blocking_pids(pid)))")
                .bind(format!("%{query}%")).bind(blocker).fetch_one(pool).await?;
            if blocked { return Ok::<_, sqlx::Error>(()); }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await??;
    Ok(())
}
