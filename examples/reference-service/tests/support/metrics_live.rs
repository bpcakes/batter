//! Live metrics export through the production executable and PostgreSQL 18.

use super::{
    ProbeResult,
    collector::{Behavior, Collector, counter, find, series},
    fixture_diagnostics::ProbeError,
    provider::{DispatchBehavior, ProviderFixture},
    startup_process::{self, AUTH_TOKEN, ExecutableChild, Signal},
};
use crate::support::profiled::initialize_schema;
use batter::{BoxError, telemetry::metrics as catalog};
use serde_json::Value;
use sqlx::{PgPool, types::Uuid};
use std::{net::SocketAddr, time::Duration};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// The configured production owner for `AUTH_TOKEN`.
const OWNER: u128 = 1;
const HTTP_LIMIT: Duration = Duration::from_secs(2);
const READY_LIMIT: Duration = Duration::from_secs(8);
const EXECUTION_LIMIT: Duration = Duration::from_secs(8);
const RUNS: u32 = 3;

pub async fn exported_command_worker_and_shutdown() {
    super::with_database_bound(bound(), |pool| async move {
        initialize_schema(&pool).await?;
        let collector = Collector::start(Behavior::Accept).await?;
        let result = served(&pool, &collector, 0).await.and_then(|()| {
            let requests = collector.requests();
            let last = requests.last().ok_or("collector received no export")?;
            final_export_is_representative(&series(last))
        });
        finish(result, collector.close().await.map_err(Into::into))
    })
    .await;
}

pub async fn collector_failure_preserves_authoritative_results() {
    super::with_database_bound(bound(), |pool| async move {
        initialize_schema(&pool).await?;
        // A refused collector: the listener is closed before the child starts.
        let refused = Collector::start(Behavior::Accept).await?;
        let endpoint = refused.endpoint();
        refused.close().await?;
        let result = served_at(&pool, &endpoint, 1).await;
        // A collector that accepts each request, then never answers it.
        let stalled = Collector::start(Behavior::StallHeaders).await?;
        let result = match result {
            Ok(()) => served(&pool, &stalled, 2).await,
            Err(error) => Err(error),
        };
        finish(result, stalled.close().await.map_err(Into::into))
    })
    .await;
}

fn bound() -> Duration {
    // Each run may use its bind, readiness, HTTP, execution and exporting reap
    // limits; headroom covers fixture setup and SQL observations.
    (ExecutableChild::lifecycle_observation_budget()
        + Duration::from_secs(8)
        + READY_LIMIT
        + HTTP_LIMIT
        + EXECUTION_LIMIT)
        * RUNS
        + Duration::from_secs(30)
}

fn finish(body: ProbeResult, cleanup: ProbeResult) -> ProbeResult {
    batter::test_support::finish(
        body.map_err(ProbeError::new),
        cleanup.map_err(ProbeError::new),
    )
    .map_err(|error| Box::new(error) as BoxError)
}

async fn served(pool: &PgPool, collector: &Collector, index: u128) -> ProbeResult {
    served_at(pool, &collector.endpoint(), index).await
}

/// One authenticated command executed by the native worker, then SIGTERM.
/// The command, durable effect and clean pool shutdown are authoritative;
/// the collector behavior must not change any of them.
async fn served_at(pool: &PgPool, collector: &str, index: u128) -> ProbeResult {
    let record = Uuid::from_u128(301 + index);
    sqlx::query("INSERT INTO reference_records (id, owner_id, generation) VALUES ($1, $2, 1)")
        .bind(record)
        .bind(Uuid::from_u128(OWNER))
        .execute(pool)
        .await?;
    let provider = ProviderFixture::start(DispatchBehavior::Accept).await?;
    let endpoint = startup_process::endpoint(pool);
    let provider_url = provider.base_url();
    let child = ExecutableChild::start_exporting_worker(
        &endpoint,
        &provider_url,
        &format!("metrics-live-{index}"),
        collector,
    )?;
    let address = child.listener();
    let observer = pool.clone();
    let observed = tokio::spawn(async move {
        tokio::time::timeout(READY_LIMIT, async {
            while !matches!(get(address, "/ready").await, Ok(200)) {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await?;
        let delivery = submit(address, record, index).await?;
        confirmed(&observer, delivery).await
    })
    .await;
    let shutdown = tokio::task::spawn_blocking(move || {
        let forbidden = startup_process::provider_forbidden_values(&endpoint, &provider_url);
        child.stop_exporting(Signal::Term, &forbidden)
    })
    .await?;
    let observed = observed
        .map_err(|error| Box::new(error) as BoxError)
        .and_then(|result| result);
    let result = finish(observed, shutdown.map_err(Into::into));
    let counts = provider.counts();
    let result = result.and_then(|()| {
        if counts.accepted_effects == 1 {
            Ok(())
        } else {
            Err(format!("provider effect was not accepted exactly once: {counts:?}").into())
        }
    });
    finish(result, provider.close().await)
}

fn final_export_is_representative(exported: &[super::collector::Series]) -> ProbeResult {
    let expect = |present: bool, what: &str| -> ProbeResult {
        if present {
            Ok(())
        } else {
            Err(format!("final export omitted {what}").into())
        }
    };
    // Probes are not operations: only the one admitted command constructs a
    // response inside the HTTP operation boundary.
    let http = [
        ("operation", "http.response_construction"),
        ("outcome", "succeeded"),
    ];
    expect(
        counter(exported, catalog::OPERATION_COMPLETIONS, &http) == Some(1),
        "the authenticated command's HTTP operation",
    )?;
    let admitted = [("admission", "root"), ("decision", "admitted")];
    expect(
        counter(exported, catalog::ADMISSION_DECISIONS, &admitted) == Some(1),
        "the command's root admission",
    )?;
    let worker = [("operation", "provider.dispatch"), ("outcome", "succeeded")];
    expect(
        counter(exported, catalog::OPERATION_COMPLETIONS, &worker) == Some(1),
        "the worker's provider operation",
    )?;
    for task in ["http", "worker"] {
        let labels = [
            ("kind", "component"),
            ("task", task),
            ("outcome", "stopped"),
        ];
        expect(
            counter(exported, catalog::TASK_EXITS, &labels) == Some(1),
            "a component task exit",
        )?;
    }
    expect(
        counter(
            exported,
            catalog::CLEANUP_HOOKS,
            &[("outcome", "succeeded")],
        )
        .is_some_and(|count| count >= 1),
        "the pool cleanup outcome",
    )?;
    let shutdown = [("cause", "requested"), ("result", "success")];
    expect(
        counter(exported, catalog::SHUTDOWNS, &shutdown) == Some(1),
        "the shutdown recorded after Stopped",
    )?;
    expect(
        find(
            exported,
            catalog::SHUTDOWN_DURATION,
            &[("result", "success")],
        )
        .is_some(),
        "the shutdown duration",
    )
}

async fn submit(address: SocketAddr, record: Uuid, index: u128) -> Result<Uuid, BoxError> {
    let body = serde_json::json!({
        "expected_generation": 1,
        "idempotency_key": format!("metrics-live-{index}"),
        "payload": {"channel": "example", "index": index},
    })
    .to_string();
    let request = format!(
        "POST /records/{record}/deliveries HTTP/1.1\r\nHost: localhost\r\n\
         Authorization: Bearer {AUTH_TOKEN}\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let (status, body) = exchange(address, &request).await?;
    if status != 202 {
        return Err(format!("authenticated command returned {status}").into());
    }
    let body: Value = serde_json::from_slice(&body)?;
    Ok(body["delivery"]["delivery_id"]
        .as_str()
        .ok_or("command response omitted its delivery")?
        .parse()?)
}

async fn confirmed(pool: &PgPool, delivery: Uuid) -> ProbeResult {
    tokio::time::timeout(EXECUTION_LIMIT, async {
        loop {
            let state: (String, String) = sqlx::query_as(
                "SELECT q.status::text, e.state
                   FROM reference_deliveries d
                   JOIN job_queue q ON q.id = d.job_id
                   JOIN reference_delivery_effects e ON e.delivery_id = d.id
                  WHERE d.id = $1",
            )
            .bind(delivery)
            .fetch_one(pool)
            .await?;
            if state == ("SUCCEEDED".to_owned(), "CONFIRMED".to_owned()) {
                return Ok::<_, sqlx::Error>(());
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await??;
    Ok(())
}

async fn get(address: SocketAddr, path: &str) -> Result<u16, BoxError> {
    let request = format!("GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");
    Ok(exchange(address, &request).await?.0)
}

async fn exchange(address: SocketAddr, request: &str) -> Result<(u16, Vec<u8>), BoxError> {
    tokio::time::timeout(HTTP_LIMIT, async {
        let mut socket = tokio::net::TcpStream::connect(address).await?;
        socket.write_all(request.as_bytes()).await?;
        let mut response = Vec::new();
        (&mut socket)
            .take(64 * 1024)
            .read_to_end(&mut response)
            .await?;
        let split = response
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .ok_or("missing HTTP header terminator")?;
        let status = std::str::from_utf8(&response[..split])?
            .split_whitespace()
            .nth(1)
            .ok_or("missing HTTP status")?
            .parse()?;
        Ok::<_, BoxError>((status, response[split + 4..].to_vec()))
    })
    .await?
}
