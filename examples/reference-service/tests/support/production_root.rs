use super::{ProbeResult, startup_process::StartupChild};
use serde_json::Value;
use sqlx::{ConnectOptions, PgPool, types::Uuid};
use std::{net::SocketAddr, time::Duration};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub async fn production_readiness(pool: PgPool) -> ProbeResult {
    let (job, original) = pending_delivery(&pool).await?;
    let final_pool = pool.clone();
    let before_shutdown = original.clone();
    let mut endpoint = pool.connect_options().to_url_lossy();
    endpoint.set_query(Some("sslmode=disable"));
    let reservation = std::net::TcpListener::bind("127.0.0.1:0")?;
    let address = reservation.local_addr()?;
    drop(reservation);
    let child = StartupChild::start_at("production", endpoint.as_str(), &address.to_string())?;
    // Join the assertion body before terminating the process, including on panic.
    let observed = tokio::spawn(async move {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if matches!(status(address, "/live").await, Ok(200)) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await?;
        assert_eq!(status(address, "/ready").await?, 503);
        // Recheck after multiple dependency sampling intervals. HTTP binding alone
        // cannot establish that readiness stays unapproved after initialization.
        tokio::time::sleep(Duration::from_millis(2100)).await;
        assert_eq!(status(address, "/live").await?, 200);
        assert_eq!(status(address, "/ready").await?, 503);
        let controls: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM job_queue WHERE job_type = 'jobs.startup.control'",
        )
        .fetch_one(&pool)
        .await?;
        assert_eq!(controls, 0, "production startup created durable controls");
        assert_eq!(
            job_snapshot(&pool, job).await?,
            before_shutdown,
            "empty production registry mutated pending delivery work"
        );
        Ok::<_, batter::BoxError>(())
    })
    .await;
    let shutdown = tokio::task::spawn_blocking(move || child.terminate()).await?;
    observed??;
    shutdown?;
    assert_eq!(
        job_snapshot(&final_pool, job).await?,
        original,
        "production shutdown mutated pending delivery work"
    );
    Ok(())
}

async fn pending_delivery(pool: &PgPool) -> Result<(Uuid, Value), batter::BoxError> {
    use batter_example_reference_service::{
        delivery::{DELIVERY_JOB_TYPE, DELIVERY_PAYLOAD_VERSION, DeliveryJobPayload},
        schema::initialize_schema,
    };
    initialize_schema(pool).await?;
    let owner = Uuid::from_u128(101);
    let payload = serde_json::to_value(DeliveryJobPayload {
        version: DELIVERY_PAYLOAD_VERSION,
        delivery_id: Uuid::from_u128(102),
        owner_id: owner,
        record_id: Uuid::from_u128(103),
        record_generation: 1,
        payload: serde_json::json!({"channel": "example"}),
    })?;
    let mut request = super::transactions::request(&payload, owner);
    request.job_type = runledger_core::jobs::JobType::new(DELIVERY_JOB_TYPE);
    let mut tx = super::transactions::read_committed(pool).await?;
    let job = runledger_postgres::jobs::enqueue_job_with_outcome_tx(&mut tx, &request)
        .await?
        .job_id;
    tx.commit().await?;
    let snapshot = job_snapshot(pool, job).await?;
    assert_eq!(snapshot["row"]["status"], "PENDING");
    assert_eq!(snapshot["attempts"], 0);
    Ok((job, snapshot))
}

async fn job_snapshot(pool: &PgPool, job: Uuid) -> Result<Value, sqlx::Error> {
    sqlx::query_scalar("SELECT jsonb_build_object('row', to_jsonb(q), 'attempts', (SELECT count(*) FROM job_attempts WHERE job_id=q.id)) FROM job_queue q WHERE id=$1")
        .bind(job).fetch_one(pool).await
}

async fn status(address: SocketAddr, path: &str) -> Result<u16, batter::BoxError> {
    tokio::time::timeout(Duration::from_millis(500), async {
        let mut socket = tokio::net::TcpStream::connect(address).await?;
        socket
            .write_all(
                format!("GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
                    .as_bytes(),
            )
            .await?;
        let mut response = Vec::new();
        (&mut socket).take(4096).read_to_end(&mut response).await?;
        let response = std::str::from_utf8(&response)?;
        let code = response
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .ok_or("missing HTTP status")?
            .parse()?;
        Ok::<_, batter::BoxError>(code)
    })
    .await?
}
