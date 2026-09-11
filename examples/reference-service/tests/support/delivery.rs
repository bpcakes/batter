use super::ProbeResult;
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Method, Request, StatusCode, header},
};
use batter::{
    cleanup::CleanupBudget,
    lifecycle::{ProcessAdmissionError, ShutdownBudget, ShutdownHandle},
    settings::SettingsSource,
};
use batter_example_reference_service::{
    config::{ConfigMode, RootSettings},
    delivery::DELIVERY_JOB_TYPE,
    http::router,
    schema::initialize_schema,
};
use serde_json::Value;
use sqlx::{PgPool, postgres::PgConnectOptions};
use std::{convert::Infallible, ffi::OsString, future::Future, time::Duration};
use tower::ServiceExt;
use uuid::Uuid;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

const OWNER_A: &str = "00000000-0000-0000-0000-000000000101";
const OWNER_B: &str = "00000000-0000-0000-0000-000000000102";
const TOKEN_A: &str = "reference-live-token-a";
const TOKEN_B: &str = "reference-live-token-b";

fn settings(owner: &str, token: &str, extra: &[(&str, &str)]) -> RootSettings {
    let mut pairs: Vec<(OsString, OsString)> = vec![
        (
            "DATABASE_URL".into(),
            "postgres://user:fake@localhost/db?sslmode=disable".into(),
        ),
        ("JOBS_WORKER_ID".into(), "reference-worker".into()),
        ("BATTER_AUTH_OWNER_ID".into(), owner.into()),
        ("BATTER_AUTH_TOKEN".into(), token.into()),
    ];
    pairs.extend(
        extra
            .iter()
            .map(|(name, value)| ((*name).into(), (*value).into())),
    );
    RootSettings::from_sources(
        ConfigMode::Serve,
        None,
        SettingsSource::default(),
        SettingsSource::from_pairs(pairs).expect("live settings names are valid"),
    )
    .expect("live settings are valid")
}

fn app(settings: &RootSettings, pool: PgPool) -> Router {
    let handle = ShutdownHandle::new();
    handle.mark_ready();
    router(settings, handle, pool).expect("validated settings build the router")
}

async fn request(
    app: &Router,
    method: Method,
    path: &str,
    token: &str,
    body: Option<&str>,
) -> TestResult<(StatusCode, Value)> {
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header(header::AUTHORIZATION, format!("Bearer {token}"));
    let body = match body {
        Some(body) => {
            builder = builder.header(header::CONTENT_TYPE, "application/json");
            Body::from(body.to_owned())
        }
        None => Body::empty(),
    };
    let response = app.clone().oneshot(builder.body(body)?).await?;
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 64 * 1024).await?;
    Ok((status, serde_json::from_slice(&bytes)?))
}

fn assert_code(status: StatusCode, body: &Value, expected_status: StatusCode, code: &str) {
    assert_eq!(status, expected_status, "unexpected response: {body}");
    assert_eq!(body["code"], code, "unexpected response: {body}");
}

struct CommandFixture {
    pool: PgPool,
    app_a: Router,
    app_b: Router,
    owner_a: Uuid,
    record_a: Uuid,
    record_b: Uuid,
}

async fn prepare_commands(pool: PgPool) -> TestResult<CommandFixture> {
    initialize_schema(&pool).await?;
    initialize_schema(&pool).await?;
    let owner_a = Uuid::parse_str(OWNER_A)?;
    let owner_b = Uuid::parse_str(OWNER_B)?;
    let record_a = Uuid::parse_str("00000000-0000-0000-0000-000000000201")?;
    let record_b = Uuid::parse_str("00000000-0000-0000-0000-000000000202")?;
    sqlx::query(
        "INSERT INTO reference_records (id, owner_id, generation)
         VALUES ($1, $2, 1), ($3, $4, 1)",
    )
    .bind(record_a)
    .bind(owner_a)
    .bind(record_b)
    .bind(owner_b)
    .execute(&pool)
    .await?;
    let app_a = app(&settings(OWNER_A, TOKEN_A, &[]), pool.clone());
    let app_b = app(&settings(OWNER_B, TOKEN_B, &[]), pool.clone());
    Ok(CommandFixture {
        pool,
        app_a,
        app_b,
        owner_a,
        record_a,
        record_b,
    })
}

const ORIGINAL_COMMAND: &str = r#"{
    "expected_generation": 1,
    "idempotency_key": "command-a",
    "payload": {"secondary": 2, "primary": 1}
}"#;
const CANONICAL_REPLAY: &str = r#"{
    "expected_generation": 1,
    "idempotency_key": "command-a",
    "payload": {"primary": 1, "secondary": 2}
}"#;

async fn initial_command(fixture: &CommandFixture) -> TestResult<Uuid> {
    let path_a = format!("/records/{}/deliveries", fixture.record_a);
    let (status, accepted) = request(
        &fixture.app_a,
        Method::POST,
        &path_a,
        TOKEN_A,
        Some(ORIGINAL_COMMAND),
    )
    .await?;
    assert_eq!(
        status,
        StatusCode::ACCEPTED,
        "unexpected response: {accepted}"
    );
    assert_eq!(accepted["outcome"], "accepted");
    assert_eq!(accepted["delivery"]["state"], "pending");
    let delivery_a = Uuid::parse_str(
        accepted["delivery"]["delivery_id"]
            .as_str()
            .ok_or("accepted response omitted delivery_id")?,
    )?;

    let (status, replayed) = request(
        &fixture.app_a,
        Method::POST,
        &path_a,
        TOKEN_A,
        Some(CANONICAL_REPLAY),
    )
    .await?;
    assert_eq!(
        status,
        StatusCode::ACCEPTED,
        "unexpected response: {replayed}"
    );
    assert_eq!(replayed["outcome"], "replayed");
    assert_eq!(replayed["delivery"]["delivery_id"], delivery_a.to_string());

    let changed = r#"{
        "expected_generation": 1,
        "idempotency_key": "command-a",
        "payload": {"primary": 999, "secondary": 2}
    }"#;
    let (status, conflict) = request(
        &fixture.app_a,
        Method::POST,
        &path_a,
        TOKEN_A,
        Some(changed),
    )
    .await?;
    assert_code(
        status,
        &conflict,
        StatusCode::CONFLICT,
        "idempotency_conflict",
    );

    let (status, observed) = request(
        &fixture.app_a,
        Method::GET,
        &format!("/deliveries/{delivery_a}"),
        TOKEN_A,
        None,
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "unexpected response: {observed}");
    assert_eq!(observed["delivery"]["delivery_id"], delivery_a.to_string());
    let (status, by_key) = request(
        &fixture.app_a,
        Method::GET,
        "/delivery-commands/command-a",
        TOKEN_A,
        None,
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "unexpected response: {by_key}");
    assert_eq!(by_key["delivery"]["delivery_id"], delivery_a.to_string());
    Ok(delivery_a)
}

async fn ownership_and_replacement(fixture: &CommandFixture, delivery_a: Uuid) -> TestResult {
    let (status, hidden) = request(
        &fixture.app_b,
        Method::GET,
        &format!("/deliveries/{delivery_a}"),
        TOKEN_B,
        None,
    )
    .await?;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "unexpected response: {hidden}"
    );
    assert_eq!(hidden["outcome"], "not_observed");

    let foreign_target = r#"{
        "expected_generation": 1,
        "idempotency_key": "foreign-target",
        "payload": {"owner": "b"}
    }"#;
    let (status, hidden_target) = request(
        &fixture.app_b,
        Method::POST,
        &format!("/records/{}/deliveries", fixture.record_a),
        TOKEN_B,
        Some(foreign_target),
    )
    .await?;
    assert_code(
        status,
        &hidden_target,
        StatusCode::NOT_FOUND,
        "record_not_observed",
    );

    let path_b = format!("/records/{}/deliveries", fixture.record_b);
    let owner_b_command = r#"{
        "expected_generation": 1,
        "idempotency_key": "command-a",
        "payload": {"owner": "b"}
    }"#;
    let (status, accepted_b) = request(
        &fixture.app_b,
        Method::POST,
        &path_b,
        TOKEN_B,
        Some(owner_b_command),
    )
    .await?;
    assert_eq!(
        status,
        StatusCode::ACCEPTED,
        "unexpected response: {accepted_b}"
    );
    assert_ne!(
        accepted_b["delivery"]["delivery_id"],
        delivery_a.to_string()
    );

    sqlx::query("UPDATE reference_records SET generation = 2 WHERE id = $1")
        .bind(fixture.record_a)
        .execute(&fixture.pool)
        .await?;
    let stale = r#"{
        "expected_generation": 1,
        "idempotency_key": "stale-command",
        "payload": {"generation": 1}
    }"#;
    let (status, stale_response) = request(
        &fixture.app_a,
        Method::POST,
        &format!("/records/{}/deliveries", fixture.record_a),
        TOKEN_A,
        Some(stale),
    )
    .await?;
    assert_code(
        status,
        &stale_response,
        StatusCode::CONFLICT,
        "stale_record_generation",
    );

    let (status, replay_after_replacement) = request(
        &fixture.app_a,
        Method::POST,
        &format!("/records/{}/deliveries", fixture.record_a),
        TOKEN_A,
        Some(CANONICAL_REPLAY),
    )
    .await?;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(replay_after_replacement["outcome"], "replayed");
    assert_eq!(
        replay_after_replacement["delivery"]["delivery_id"],
        delivery_a.to_string()
    );

    discard_and_reconcile(fixture).await
}

async fn discard_and_reconcile(fixture: &CommandFixture) -> TestResult {
    let lost_ack = r#"{
        "expected_generation": 2,
        "idempotency_key": "discarded-response",
        "payload": {"generation": 2}
    }"#;
    let discarded = request(
        &fixture.app_a,
        Method::POST,
        &format!("/records/{}/deliveries", fixture.record_a),
        TOKEN_A,
        Some(lost_ack),
    )
    .await?;
    assert_eq!(discarded.0, StatusCode::ACCEPTED);
    drop(discarded);
    let (status, reconciled) = request(
        &fixture.app_a,
        Method::GET,
        "/delivery-commands/discarded-response",
        TOKEN_A,
        None,
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "unexpected response: {reconciled}");
    assert_eq!(reconciled["outcome"], "observed");
    assert_eq!(reconciled["delivery"]["generation"], 2);
    Ok(())
}

#[derive(sqlx::FromRow)]
struct JobEvidence {
    delivery_id: Uuid,
    command_owner: Uuid,
    command_key: String,
    job_owner: Uuid,
    job_key: String,
    job_type: String,
    status: String,
    payload: Value,
}

async fn assert_persistence(fixture: &CommandFixture) -> TestResult {
    let counts: (i64, i64, i64, i64) = sqlx::query_as(
        "SELECT
            (SELECT count(*) FROM reference_delivery_commands),
            (SELECT count(*) FROM reference_deliveries),
            (SELECT count(*) FROM job_queue),
            (SELECT count(*) FROM job_events WHERE event_type = 'ENQUEUED')",
    )
    .fetch_one(&fixture.pool)
    .await?;
    assert_eq!(counts, (3, 3, 3, 3));
    let rejected_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM reference_delivery_commands
          WHERE (owner_id = $1 AND idempotency_key = 'stale-command')
             OR idempotency_key = 'foreign-target'",
    )
    .bind(fixture.owner_a)
    .fetch_one(&fixture.pool)
    .await?;
    assert_eq!(rejected_count, 0);

    let rows: Vec<JobEvidence> = sqlx::query_as(
        "SELECT d.id AS delivery_id, c.owner_id AS command_owner,
                c.enqueue_idempotency_key AS command_key,
                q.organization_id AS job_owner, q.idempotency_key AS job_key,
                q.job_type, q.status::text, q.payload
           FROM reference_deliveries d
           JOIN reference_delivery_commands c
             ON c.owner_id = d.owner_id AND c.idempotency_key = d.idempotency_key
           JOIN job_queue q ON q.id = d.job_id
          ORDER BY d.id",
    )
    .fetch_all(&fixture.pool)
    .await?;
    assert_eq!(rows.len(), 3);
    for row in rows {
        assert_eq!(row.command_owner, row.job_owner);
        assert_eq!(row.command_key, format!("delivery:{}", row.delivery_id));
        assert_eq!(row.job_key, row.command_key);
        assert_eq!(row.job_type, DELIVERY_JOB_TYPE);
        assert_eq!(row.status, "PENDING");
        assert_eq!(row.payload["delivery_id"], row.delivery_id.to_string());
        assert_eq!(row.payload["owner_id"], row.command_owner.to_string());
    }
    Ok(())
}

pub async fn command_and_reconciliation(pool: PgPool) -> ProbeResult {
    let fixture = prepare_commands(pool).await?;
    let delivery_id = initial_command(&fixture).await?;
    ownership_and_replacement(&fixture, delivery_id).await?;
    assert_persistence(&fixture).await
}

async fn failure_code(
    app: &Router,
    path: &str,
    token: &str,
) -> Result<(StatusCode, String), Box<dyn std::error::Error + Send + Sync>> {
    let (status, body) = request(app, Method::GET, path, token, None).await?;
    Ok((
        status,
        body["code"]
            .as_str()
            .ok_or("infrastructure response omitted code")?
            .to_owned(),
    ))
}

fn shutdown_budget() -> ShutdownBudget {
    let short = Duration::from_secs(1);
    ShutdownBudget::new(
        short,
        short,
        short,
        CleanupBudget::new(short, short, short).expect("static cleanup budget is valid"),
    )
    .expect("static shutdown budget is valid")
}

async fn assert_pool_timeout(options: PgConnectOptions, missing: &str) -> TestResult {
    let pool_timeout_settings = settings(
        OWNER_A,
        TOKEN_A,
        &[
            ("BATTER_POOL_MAX_CONNECTIONS", "1"),
            ("BATTER_POOL_ACQUIRE_TIMEOUT_MS", "500"),
            ("BATTER_REQUEST_TIMEOUT_MS", "1500"),
        ],
    );
    let pool_timeout = pool_timeout_settings
        .pool_options()
        .connect_with(options)
        .await?;
    let held = pool_timeout.acquire().await?;
    let pool_timeout_app = app(&pool_timeout_settings, pool_timeout.clone());
    let outcome = failure_code(&pool_timeout_app, missing, TOKEN_A).await?;
    assert_eq!(
        outcome,
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "service_unavailable".to_owned()
        )
    );
    drop(held);
    pool_timeout.close().await;
    Ok(())
}

async fn assert_request_deadline(options: PgConnectOptions, missing: &str) -> TestResult {
    let deadline_settings = settings(
        OWNER_A,
        TOKEN_A,
        &[
            ("BATTER_POOL_MAX_CONNECTIONS", "1"),
            ("BATTER_POOL_ACQUIRE_TIMEOUT_MS", "1500"),
            ("BATTER_REQUEST_TIMEOUT_MS", "30"),
        ],
    );
    let deadline_pool = deadline_settings
        .pool_options()
        .connect_with(options)
        .await?;
    let held = deadline_pool.acquire().await?;
    let deadline_app = app(&deadline_settings, deadline_pool.clone());
    let outcome = failure_code(&deadline_app, missing, TOKEN_A).await?;
    assert_eq!(
        outcome,
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "deadline_exceeded".to_owned()
        )
    );
    drop(held);
    deadline_pool.close().await;
    Ok(())
}

async fn assert_bulkhead(options: PgConnectOptions, missing: &str) -> TestResult {
    let bulkhead_settings = settings(
        OWNER_A,
        TOKEN_A,
        &[
            ("BATTER_POOL_MAX_CONNECTIONS", "1"),
            ("BATTER_POOL_ACQUIRE_TIMEOUT_MS", "1500"),
            ("BATTER_REQUEST_TIMEOUT_MS", "2000"),
            ("BATTER_BULKHEAD_CAPACITY", "1"),
        ],
    );
    let bulkhead_pool = bulkhead_settings
        .pool_options()
        .connect_with(options)
        .await?;
    let held = bulkhead_pool.acquire().await?;
    let bulkhead_app = app(&bulkhead_settings, bulkhead_pool.clone());
    let mut first = Box::pin(request(&bulkhead_app, Method::GET, missing, TOKEN_A, None));
    let pending = std::future::poll_fn(|context| {
        std::task::Poll::Ready(first.as_mut().poll(context).is_pending())
    })
    .await;
    assert!(
        pending,
        "first request did not hold configured bulkhead capacity"
    );
    let outcome = failure_code(&bulkhead_app, missing, TOKEN_A).await?;
    assert_eq!(
        outcome,
        (StatusCode::SERVICE_UNAVAILABLE, "overloaded".to_owned())
    );
    drop(held);
    let (status, body) = first.await?;
    assert_eq!(status, StatusCode::NOT_FOUND, "unexpected response: {body}");
    assert_eq!(body["outcome"], "not_observed");
    bulkhead_pool.close().await;
    Ok(())
}

async fn assert_process_capacity() -> TestResult {
    let process_settings = settings(OWNER_A, TOKEN_A, &[("BATTER_PROCESS_CAPACITY", "1")]);
    let mut supervisor = process_settings.supervisor(shutdown_budget())?;
    supervisor.register("initialized", |signal| async move {
        signal.mark_started();
        signal.draining().await;
        Ok(())
    })?;
    let process = supervisor
        .process_handle()
        .ok_or("configured supervisor omitted process admission")?;
    let handle = supervisor.handle();
    handle.mark_ready();
    let running = supervisor.start();
    handle
        .wait_ready()
        .await
        .map_err(|_| "supervisor did not become ready")?;
    let (release, held) = tokio::sync::oneshot::channel();
    let receipt = process.try_spawn("held", |_| async move {
        held.await.expect("test retains release sender");
        Ok::<_, Infallible>(())
    })?;
    assert!(matches!(
        process.try_spawn("excess", |_| async { Ok::<_, Infallible>(()) }),
        Err(ProcessAdmissionError::Full)
    ));
    release.send(()).expect("held task still receives release");
    receipt.wait().await?;
    handle.request();
    let report = running.wait().await?;
    assert!(report.is_success());
    assert_eq!(report.completed_process_tasks, 1);
    Ok(())
}

pub async fn configured_root_bounds(fixture_pool: PgPool) -> ProbeResult {
    initialize_schema(&fixture_pool).await?;
    let options = (*fixture_pool.connect_options()).clone();
    let missing = "/deliveries/00000000-0000-0000-0000-000000000301";
    assert_pool_timeout(options.clone(), missing).await?;
    assert_request_deadline(options.clone(), missing).await?;
    assert_bulkhead(options, missing).await?;
    assert_process_capacity().await
}
