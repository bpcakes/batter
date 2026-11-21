//! The real boundary, driven in process with test flags and a database handle
//! that never connects: every case here is answered before any SQL runs, or
//! asserts that the database being unreachable is reported as such.

use std::time::Duration;

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use batter::{
    axum::{InProcessClient, ResponseConstructionBudget},
    flags::{core::Toggle, test::TestFlags},
    lifecycle::{ShutdownBudget, ShutdownHandle, Supervisor},
    runledger::{PgSessionProfile, RunledgerDatabase},
};
use serde_json::Value;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};

use super::{AppState, assemble};

enum Halt {
    Engaged,
    Released,
}

/// A profiled pool aimed at a closed loopback port, acquiring for at most
/// 200 ms. Its cleanup slot belongs to a supervisor that is never started.
fn unreachable_database() -> RunledgerDatabase {
    let second = Duration::from_secs(1);
    let cleanup = batter::cleanup::CleanupBudget::new(second, second, second).unwrap();
    let mut supervisor =
        Supervisor::new(ShutdownBudget::new(second, second, second, cleanup).unwrap());
    let profile = PgSessionProfile::new(
        "checkout",
        "checkout",
        vec!["public".into()],
        Duration::ZERO,
        Duration::ZERO,
    )
    .unwrap();
    let options = PgConnectOptions::new_without_pgpass()
        .host("127.0.0.1")
        .port(1)
        .username("checkout")
        .database("checkout");
    let pool = batter::sqlx::pool_in(
        supervisor.reserve_cleanup("database").unwrap(),
        PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(Duration::from_millis(200)),
        options,
        profile,
    )
    .unwrap();
    RunledgerDatabase::from_profiled(pool).unwrap()
}

async fn client(halt: Halt, express: Toggle) -> InProcessClient {
    let declared = crate::flags::declare().unwrap();
    let handles = declared.handles;
    let builder = TestFlags::builder(declared.catalog, declared.serving)
        .unwrap()
        .set(&handles.express_receipt, express);
    let fixture = match halt {
        Halt::Engaged => builder.engage(&handles.checkout_halt),
        Halt::Released => builder.release(&handles.checkout_halt),
    }
    .build()
    .unwrap();
    let (control, approval) = ShutdownHandle::new_with_readiness_approval();
    approval.approve();
    let gate = fixture.flags().http_gate(&handles.checkout_halt).unwrap();
    let state = AppState {
        database: unreachable_database(),
        express_receipt: handles.express_receipt.clone(),
    };
    assemble(
        control.status(),
        control.operation_admission(),
        ResponseConstructionBudget::new(Duration::from_secs(2)).unwrap(),
        gate,
        state,
    )
    .await
    .unwrap()
    .in_process()
}

async fn send(
    client: &InProcessClient,
    method: &str,
    path: &str,
    body: &str,
) -> (StatusCode, Value) {
    let request = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json")
        .body(Body::from(body.to_owned()))
        .unwrap();
    let response = client.request(request).await;
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

const ORDER: &str = r#"{"order_ref":"order-1","recipient":"buyer@example.test"}"#;

#[tokio::test]
async fn an_engaged_halt_disables_the_whole_checkout_group_but_not_the_probes() {
    let client = client(Halt::Engaged, Toggle::On).await;
    let (status, body) = send(&client, "POST", "/checkout", ORDER).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["code"], "disabled", "{body}");
    let (status, body) = send(&client, "GET", "/checkout/status", "").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["code"], "disabled", "{body}");
    let (status, _) = send(&client, "GET", "/live", "").await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn a_released_halt_serves_the_group_from_the_request_view() {
    let client = client(Halt::Released, Toggle::On).await;
    let (status, body) = send(&client, "GET", "/checkout/status", "").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["accepting"], true);
    assert_eq!(body["express_receipt"], "on");

    let (status, body) = send(
        &client,
        "POST",
        "/checkout",
        r#"{"order_ref":"no spaces","recipient":"x@y.z"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"], "invalid_request");
}

#[tokio::test]
async fn an_unreachable_database_is_reported_as_unavailable_not_accepted() {
    let client = client(Halt::Released, Toggle::Off).await;
    let (status, body) = send(&client, "POST", "/checkout", ORDER).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert_ne!(body["code"], "disabled", "{body}");
    assert_ne!(body["status"], "accepted", "{body}");
}
