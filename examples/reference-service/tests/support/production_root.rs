use super::{
    ProbeResult,
    fixture_diagnostics::ProbeError,
    startup_process::{self, Signal, StartupChild},
};
use batter::BoxError;
use serde_json::Value;
use sqlx::{PgPool, types::Uuid};
use std::{net::SocketAddr, time::Duration};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const HTTP_RESPONSE_MAX_BYTES: u64 = 64 * 1024;

pub async fn production_readiness(pool: PgPool) -> ProbeResult {
    let (job, original) = pending_delivery(&pool).await?;
    for signal in [Signal::Term, Signal::Int] {
        served_until(&pool, job, &original, signal).await?;
    }
    Ok(())
}

async fn served_until(pool: &PgPool, job: Uuid, original: &Value, signal: Signal) -> ProbeResult {
    let observer = pool.clone();
    let before_shutdown = original.clone();
    let endpoint = startup_process::endpoint(pool);
    let reservation = std::net::TcpListener::bind("127.0.0.1:0")?;
    let address = reservation.local_addr()?;
    drop(reservation);
    let child = StartupChild::start_at("production", &endpoint, &address.to_string())?;
    // Join the assertion body before signalling the process, including on panic.
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
        let response = response(address, "/delivery-commands/transport-probe").await?;
        assert_eq!(
            response.status, 401,
            "production business route did not reach authentication"
        );
        let request_id = response
            .request_id
            .as_deref()
            .ok_or("production response omitted x-request-id")?;
        let body: Value = serde_json::from_slice(&response.body)?;
        assert_eq!(body["code"], "authentication_required");
        assert_eq!(body["request_id"].as_str(), Some(request_id));
        assert_eq!(status(address, "/ready").await?, 503);
        // Recheck after multiple dependency sampling intervals. HTTP binding alone
        // cannot establish that readiness stays unapproved after initialization.
        tokio::time::sleep(Duration::from_millis(2100)).await;
        assert_eq!(status(address, "/live").await?, 200);
        assert_eq!(status(address, "/ready").await?, 503);
        assert_eq!(
            controls(&observer).await?,
            0,
            "production startup created durable controls"
        );
        assert_eq!(
            job_snapshot(&observer, job).await?,
            before_shutdown,
            "empty production registry mutated pending delivery work"
        );
        Ok::<_, batter::BoxError>(())
    })
    .await;
    // The child reports success only after run's checked shutdown: joined native
    // work and the successful application-owned postgres.pool record.
    let shutdown =
        tokio::task::spawn_blocking(move || child.stop(signal, &[], &[&endpoint, "fixture-token"]))
            .await?;
    let observed = observed
        .map_err(|error| Box::new(error) as BoxError)
        .and_then(|result| result);
    let shutdown = shutdown.map_err(|error| -> BoxError { error.into() });
    finish_results(observed, shutdown)?;
    assert_eq!(
        controls(pool).await?,
        0,
        "production shutdown created durable controls"
    );
    assert_eq!(
        job_snapshot(pool, job).await?,
        *original,
        "production shutdown mutated pending delivery work"
    );
    Ok(())
}

fn finish_results(body: ProbeResult, shutdown: ProbeResult) -> ProbeResult {
    batter_test_support::finish(
        body.map_err(ProbeError::new),
        shutdown.map_err(ProbeError::new),
    )
    .map_err(|error| Box::new(error) as BoxError)
}

pub(super) fn process_contracts() {
    let failure = finish_results(
        Err(std::io::Error::other("assertion marker").into()),
        Err(std::io::Error::other("shutdown marker").into()),
    )
    .expect_err("both settled failures must remain inspectable");
    let batter_test_support::TestFailure::Both { body, cleanup } = failure
        .downcast_ref::<batter_test_support::TestFailure<ProbeError, ProbeError>>()
        .expect("production result uses the shared dual-failure container")
    else {
        panic!("both branches were not retained")
    };
    assert_eq!(body.0.to_string(), "assertion marker");
    assert_eq!(cleanup.0.to_string(), "shutdown marker");
}

async fn controls(pool: &PgPool) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT count(*) FROM job_queue WHERE job_type = 'jobs.startup.control'")
        .fetch_one(pool)
        .await
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
    Ok(response(address, path).await?.status)
}

struct RawResponse {
    status: u16,
    request_id: Option<String>,
    body: Vec<u8>,
}

async fn response(address: SocketAddr, path: &str) -> Result<RawResponse, batter::BoxError> {
    tokio::time::timeout(Duration::from_millis(500), async {
        let mut socket = tokio::net::TcpStream::connect(address).await?;
        socket
            .write_all(
                format!("GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
                    .as_bytes(),
            )
            .await?;
        let mut response = Vec::new();
        (&mut socket)
            .take(HTTP_RESPONSE_MAX_BYTES + 1)
            .read_to_end(&mut response)
            .await?;
        if response.len() > HTTP_RESPONSE_MAX_BYTES as usize {
            return Err(format!(
                "HTTP response exceeded the {HTTP_RESPONSE_MAX_BYTES}-byte capture limit"
            )
            .into());
        }
        let response = std::str::from_utf8(&response)?;
        let (headers, body) = response
            .split_once("\r\n\r\n")
            .ok_or("missing HTTP header terminator")?;
        let status = headers
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .ok_or("missing HTTP status")?
            .parse()?;
        let request_id = headers.lines().skip(1).find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("x-request-id")
                .then(|| value.trim().to_owned())
        });
        Ok::<_, batter::BoxError>(RawResponse {
            status,
            request_id,
            body: body.as_bytes().to_vec(),
        })
    })
    .await?
}
