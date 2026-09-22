use super::{
    ProbeResult,
    fixture_diagnostics::ProbeError,
    provider::{DispatchBehavior, ProviderFixture},
    startup_process::{self, ExecutableChild, Signal},
};
use crate::support::profiled::initialize_schema;
use batter::{BoxError, operation::OperationContext};
use batter_example_reference_service::delivery::{OwnerId, SubmitDelivery};
use serde_json::Value;
use sqlx::{Connection, PgConnection, PgPool, types::Uuid};
use std::{net::SocketAddr, time::Duration};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const HTTP_RESPONSE_MAX_BYTES: u64 = 64 * 1024;
const HTTP_RESPONSE_LIMIT: Duration = Duration::from_millis(500);
const INITIAL_OBSERVATION_LIMIT: Duration = Duration::from_secs(5);
const READINESS_TRANSITION_LIMIT: Duration = Duration::from_secs(8);
const FRESHNESS_WITNESS_WAIT: Duration = Duration::from_millis(3_200);
const SUBMISSION_LIMIT: Duration = Duration::from_secs(5);
const FIXTURE_HEADROOM: Duration = Duration::from_secs(30);
const SIGNALS: [Signal; 2] = [Signal::Term, Signal::Int];

// The scenario owns fixture selection: its serial phases exceed the generic
// fixture's 30-second observation budget even before process settlement.
pub async fn production_readiness() {
    super::with_database_bound(completion_bound(), readiness_body).await;
}

fn completion_bound() -> Duration {
    let per_signal = INITIAL_OBSERVATION_LIMIT * 3
        + READINESS_TRANSITION_LIMIT * 2
        + FRESHNESS_WITNESS_WAIT
        + HTTP_RESPONSE_LIMIT * 2
        + SUBMISSION_LIMIT
        + ExecutableChild::lifecycle_observation_budget();
    // Headroom covers fixture setup, SQL observations and settlement. This is a
    // diagnostic wait bound, not a proof that arbitrary database I/O terminates.
    per_signal * SIGNALS.len() as u32
        + startup_process::announcement_failure_budget()
        + FIXTURE_HEADROOM
}

async fn readiness_body(pool: PgPool) -> ProbeResult {
    initialize_schema(&pool).await?;
    let endpoint = startup_process::endpoint(&pool);
    tokio::task::spawn_blocking(move || startup_process::check_announcement_failure(&endpoint))
        .await?
        .map_err(|error| -> BoxError { error.into() })?;
    let fixture = ProviderFixture::start(DispatchBehavior::Accept).await?;
    let result = async {
        for (index, signal) in SIGNALS.into_iter().enumerate() {
            let job = pending_delivery(&pool, index as u128).await?;
            served_until(&pool, job, signal, &fixture).await?;
        }
        let counts = fixture.counts();
        if counts.dispatch_requests != 2 || counts.accepted_effects != 2 {
            return Err(
                format!("production worker did not execute both effects: {counts:?}").into(),
            );
        }
        Ok(())
    }
    .await;
    let cleanup = fixture.close().await;
    finish_results(result, cleanup)
}

async fn served_until(
    pool: &PgPool,
    job: Uuid,
    signal: Signal,
    fixture: &ProviderFixture,
) -> ProbeResult {
    let observer = pool.clone();
    let endpoint = startup_process::endpoint(pool);
    let provider_url = fixture.base_url();
    let child = ExecutableChild::start_provider_worker(
        &endpoint,
        &provider_url,
        &format!("production-readiness-{signal:?}"),
    )?;
    let address = child.listener();
    // Join the assertion body before signalling the process, including on panic.
    let observed = tokio::spawn(async move {
        tokio::time::timeout(INITIAL_OBSERVATION_LIMIT, async {
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
        tokio::time::timeout(INITIAL_OBSERVATION_LIMIT, async {
            loop {
                if matches!(status(address, "/ready").await, Ok(200)) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await?;
        assert_eq!(
            controls(&observer).await?,
            0,
            "production startup created durable controls"
        );
        tokio::time::timeout(INITIAL_OBSERVATION_LIMIT, async {
            loop {
                let state: (String, String) = sqlx::query_as(
                    "SELECT q.status::text, e.state
                       FROM job_queue q
                       JOIN reference_deliveries d ON d.job_id = q.id
                       JOIN reference_delivery_effects e ON e.delivery_id = d.id
                      WHERE q.id = $1",
                )
                .bind(job)
                .fetch_one(&observer)
                .await?;
                if state == ("SUCCEEDED".to_owned(), "CONFIRMED".to_owned()) {
                    break Ok::<_, sqlx::Error>(());
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await??;
        // Longer than the three-second health freshness limit: a monitor that
        // stops after its first successful observation cannot pass this check.
        tokio::time::sleep(FRESHNESS_WITNESS_WAIT).await;
        assert_eq!(status(address, "/ready").await?, 200);
        readiness_recovers_after_database_outage(&observer, address).await?;
        Ok::<_, batter::BoxError>(())
    })
    .await;
    // The child reports success only after run's checked shutdown: joined native
    // work and the successful application-owned postgres.pool record.
    let shutdown = tokio::task::spawn_blocking(move || {
        let forbidden = startup_process::provider_forbidden_values(&endpoint, &provider_url);
        child.stop(signal, &forbidden)
    })
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
    Ok(())
}

fn finish_results(body: ProbeResult, shutdown: ProbeResult) -> ProbeResult {
    batter::test_support::finish(
        body.map_err(ProbeError::new),
        shutdown.map_err(ProbeError::new),
    )
    .map_err(|error| Box::new(error) as BoxError)
}

async fn readiness_recovers_after_database_outage(
    pool: &PgPool,
    address: SocketAddr,
) -> ProbeResult {
    // This case owns one disposable database. PostgreSQL requires altering
    // connection admission from a different database; keep that control alive.
    let database: String = sqlx::query_scalar("SELECT current_database()")
        .fetch_one(pool)
        .await?;
    let options = pool.connect_options().as_ref().clone().database("postgres");
    let mut control = PgConnection::connect_with(&options).await?;
    let observed = database_outage(&mut control, &database, address).await;
    let closed = control
        .close()
        .await
        .map_err(|error| Box::new(error) as BoxError);
    finish_results(observed, closed)
}

async fn database_outage(
    control: &mut PgConnection,
    database: &str,
    address: SocketAddr,
) -> ProbeResult {
    let quoted = format!("\"{}\"", database.replace('"', "\"\""));
    // Identifier comes from current_database and is quoted with doubled quotes;
    // PostgreSQL does not support binding an identifier in ALTER DATABASE.
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "ALTER DATABASE {quoted} ALLOW_CONNECTIONS false"
    )))
    .execute(&mut *control)
    .await?;
    let observed = async {
        sqlx::query(
            "SELECT pg_terminate_backend(pid) FROM pg_stat_activity
            WHERE datname = $1 AND backend_type = 'client backend'",
        )
        .bind(database)
        .execute(&mut *control)
        .await?;
        wait_for_readiness(address, 503).await
    }
    .await;
    let restored = sqlx::query(sqlx::AssertSqlSafe(format!(
        "ALTER DATABASE {quoted} ALLOW_CONNECTIONS true"
    )))
    .execute(&mut *control)
    .await
    .map(|_| ())
    .map_err(|error| Box::new(error) as BoxError);
    finish_results(observed, restored)?;
    wait_for_readiness(address, 200).await
}

async fn wait_for_readiness(address: SocketAddr, expected: u16) -> ProbeResult {
    tokio::time::timeout(READINESS_TRANSITION_LIMIT, async {
        loop {
            if status(address, "/ready").await? == expected {
                return Ok::<_, BoxError>(());
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await??;
    Ok(())
}

pub(super) fn process_contracts() {
    // Independent minimum: both cases may legitimately use every phase limit.
    let serial_phases = Duration::from_millis(68_400);
    assert!(serial_phases > Duration::from_secs(30));
    assert!(
        completion_bound()
            >= serial_phases
                + ExecutableChild::lifecycle_observation_budget() * 2
                + startup_process::announcement_failure_budget()
                + FIXTURE_HEADROOM
    );
    let failure = finish_results(
        Err(std::io::Error::other("assertion marker").into()),
        Err(std::io::Error::other("shutdown marker").into()),
    )
    .expect_err("both settled failures must remain inspectable");
    let batter::test_support::TestFailure::Both { body, cleanup } = failure
        .downcast_ref::<batter::test_support::TestFailure<ProbeError, ProbeError>>()
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

async fn pending_delivery(pool: &PgPool, index: u128) -> Result<Uuid, batter::BoxError> {
    let owner = OwnerId::new(Uuid::from_u128(101))?;
    let record = Uuid::from_u128(103 + index);
    sqlx::query("INSERT INTO reference_records (id, owner_id, generation) VALUES ($1, $2, 1)")
        .bind(record)
        .bind(owner.as_uuid())
        .execute(pool)
        .await?;
    let context = batter::operation::OperationOwner::new(SUBMISSION_LIMIT)?.into_context();
    let submitted = crate::support::profiled::submit(
        pool,
        &context,
        owner,
        record,
        SubmitDelivery {
            expected_generation: 1,
            idempotency_key: format!("production-readiness-{index}"),
            payload: serde_json::json!({"channel": "example", "index": index}),
        },
    )
    .await?;
    Ok(
        sqlx::query_scalar("SELECT job_id FROM reference_deliveries WHERE id = $1")
            .bind(submitted.delivery.delivery_id)
            .fetch_one(pool)
            .await?,
    )
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
    tokio::time::timeout(HTTP_RESPONSE_LIMIT, async {
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
