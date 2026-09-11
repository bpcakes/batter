use std::time::Duration;

use batter_sqlx::test_support::{
    CleanupPhase, ConnectionPlan, FixtureError, FixtureSuite, SessionObserver,
};
use sqlx::{PgPool, postgres::PgPoolOptions};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    sync::oneshot,
};

use super::{
    ProbeResult,
    fixture_observation::{BOUND, admin},
    fixture_run,
};

// This negative control intentionally keeps a connection producer alive through
// body exit. It proves why an absence witness cannot replace stopping producers.
pub async fn startup_is_not_a_connection_fence() -> ProbeResult {
    let catalog = admin().await?;
    let session_pool = admin().await?;
    let control = SessionObserver::new(session_pool.clone(), BOUND)?;
    let (send, receive) = oneshot::channel();
    let (release, gate) = oneshot::channel();
    let mut run = FixtureSuite::new(super::harness().await?)
        .with_session_observer(control)
        .start(move |scope| {
            Box::pin(async move {
                let plan = ConnectionPlan::new(vec![PgPoolOptions::new().max_connections(1)], 1)?;
                let db = scope.empty(&plan).await?;
                send.send(db.database_name().to_owned()).unwrap();
                gate.await.unwrap();
                Ok::<_, FixtureError>(())
            })
        });
    let name = tokio::time::timeout(BOUND, receive).await??;
    // This probe requires the dedicated serial server: container proxies may
    // rewrite the client's port, so socket.local_addr() cannot identify the
    // server-side peer. No other connection producer may run in this window.
    let before = starting_pids(&catalog).await?;
    let mut socket = challenge(&catalog, &name).await?;
    let after = starting_pids(&catalog).await?;
    let created: Vec<_> = after
        .into_iter()
        .filter(|pid| !before.contains(pid))
        .collect();
    assert_eq!(
        created.len(),
        1,
        "SCRAM challenge acknowledges one unassigned backend"
    );
    release.send(()).unwrap();
    super::fixture_sessions::phase(&run, CleanupPhase::CleaningLease).await;
    // Observation has passed the unassigned backend. Witness the actual DROP's
    // process-signal barrier, not merely an arbitrary slow cleanup operation.
    let drop_pid = drop_barrier(&catalog, &name).await?;
    let pending = run
        .wait_for(Duration::from_millis(30))
        .await
        .unwrap()
        .is_none();
    let barrier_retained = sqlx::query_scalar::<_, bool>(
        "SELECT COALESCE(wait_event = 'ProcSignalBarrier', false) FROM pg_stat_activity WHERE pid = $1",
    )
    .bind(drop_pid)
    .fetch_optional(&catalog)
    .await?
    .unwrap_or(false);
    let still_starting = starting_pids(&catalog).await?.contains(&created[0]);
    socket.shutdown().await?;
    drop(socket);
    tokio::time::timeout(BOUND, async {
        while starting_pids(&catalog).await?.contains(&created[0]) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        Ok::<_, sqlx::Error>(())
    })
    .await??;
    let report = tokio::time::timeout(BOUND, run.into_report()).await??;
    let absent = fixture_run::absent(&catalog, &name).await;
    tokio::join!(session_pool.close(), catalog.close());
    absent?;
    assert!(report.is_ok() && still_starting && pending && barrier_retained);
    Ok(())
}

async fn starting_pids(pool: &PgPool) -> Result<Vec<i32>, sqlx::Error> {
    sqlx::query_scalar("SELECT pid FROM pg_stat_activity WHERE backend_type = 'client backend' AND datname IS NULL")
        .fetch_all(pool).await
}

async fn drop_barrier(
    pool: &PgPool,
    database: &str,
) -> Result<i32, Box<dyn std::error::Error + Send + Sync>> {
    Ok(tokio::time::timeout(BOUND, async {
        loop {
            let pid = sqlx::query_scalar(
                "SELECT pid FROM pg_stat_activity WHERE pid <> pg_backend_pid()
                 AND query LIKE 'DROP DATABASE%' AND position($1 in query) > 0
                 AND wait_event = 'ProcSignalBarrier'",
            )
            .bind(database)
            .fetch_optional(pool)
            .await?;
            if let Some(pid) = pid {
                return Ok::<_, sqlx::Error>(pid);
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await??)
}

async fn challenge(
    pool: &PgPool,
    database: &str,
) -> Result<TcpStream, Box<dyn std::error::Error + Send + Sync>> {
    let options = pool.connect_options();
    let mut socket = TcpStream::connect((options.get_host(), options.get_port())).await?;
    let mut packet = 196608_u32.to_be_bytes().to_vec();
    for field in ["user", options.get_username(), "database", database] {
        packet.extend_from_slice(field.as_bytes());
        packet.push(0);
    }
    packet.push(0);
    let length = u32::try_from(packet.len() + 4)?;
    socket.write_all(&length.to_be_bytes()).await?;
    socket.write_all(&packet).await?;
    let mut header = [0; 9];
    tokio::time::timeout(BOUND, socket.read_exact(&mut header)).await??;
    assert_eq!(header[0], b'R', "server must request authentication");
    assert_eq!(
        &header[5..],
        &10_u32.to_be_bytes(),
        "startup control requires SCRAM host authentication"
    );
    Ok(socket)
}
