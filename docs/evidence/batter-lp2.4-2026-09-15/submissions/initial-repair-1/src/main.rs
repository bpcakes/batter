use axum::{
    Extension, Router,
    extract::State,
    http::StatusCode,
    middleware,
    response::{IntoResponse, Response},
    routing::get,
};
use batter::{
    BoxError,
    cleanup::CleanupBudget,
    lifecycle::{SharedShutdownReport, ShutdownBudget, Supervisor},
    operation::OperationContext,
    startup::Startup,
};
use batter_axum::{RequestPolicy, ResponseConstructionBudget, register_http_in, request_admission};
use batter_sqlx::{PgLease, SqlxFailure, pool_in};
use sqlx::{PgPool, postgres::{PgConnectOptions, PgPoolOptions}};
use std::{
    env,
    fmt,
    process::ExitCode,
    str::FromStr,
    time::Duration,
};
use tokio::{net::TcpListener, sync::{mpsc, oneshot}};

const OPERATION_BUDGET: Duration = Duration::from_secs(5);

#[derive(Clone)]
struct ApplicationState {
    pool: PgPool,
    failure: mpsc::Sender<()>,
}

#[derive(Clone, Copy)]
struct ApplicationFailure;

impl fmt::Debug for ApplicationFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ApplicationFailure")
    }
}

impl fmt::Display for ApplicationFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("application failure")
    }
}

impl std::error::Error for ApplicationFailure {}

async fn select_42(pool: &PgPool, context: &OperationContext) -> Result<i32, ApplicationFailure> {
    let mut lease = PgLease::acquire(pool, context)
        .await
        .map_err(|_| ApplicationFailure)?;
    context
        .run("database.select_42", move |_| async move {
            let value = sqlx::query_scalar::<_, i32>("SELECT 42")
                .fetch_one(lease.connection())
                .await
                .map_err(SqlxFailure::from)?;
            lease.return_to_pool();
            Ok::<i32, SqlxFailure>(value)
        })
        .await
        .map_err(|_| ApplicationFailure)
}

async fn query_absent_relation(
    pool: &PgPool,
    context: &OperationContext,
) -> Result<(), ApplicationFailure> {
    let mut lease = PgLease::acquire(pool, context)
        .await
        .map_err(|_| ApplicationFailure)?;
    context
        .run("database.expected_failure", move |_| async move {
            sqlx::query("SELECT 1 FROM consumer_definitely_absent_relation")
                .execute(lease.connection())
                .await
                .map_err(SqlxFailure::from)?;
            lease.return_to_pool();
            Ok::<(), SqlxFailure>(())
        })
        .await
        .map_err(|_| ApplicationFailure)
}

async fn query(
    State(state): State<ApplicationState>,
    Extension(context): Extension<OperationContext>,
) -> Response {
    match select_42(&state.pool, &context).await {
        Ok(42) => (StatusCode::OK, "42").into_response(),
        _ => (StatusCode::INTERNAL_SERVER_ERROR, "internal error").into_response(),
    }
}

async fn trigger_failure(State(state): State<ApplicationState>) -> Response {
    match state.failure.send(()).await {
        Ok(()) => (StatusCode::ACCEPTED, "accepted").into_response(),
        Err(_) => (StatusCode::SERVICE_UNAVAILABLE, "unavailable").into_response(),
    }
}

async fn run() -> Result<SharedShutdownReport, ApplicationFailure> {
    let second = Duration::from_secs(1);
    let cleanup = CleanupBudget::new(
        Duration::from_secs(5),
        second,
        Duration::from_secs(6),
    )
    .map_err(|_| ApplicationFailure)?;
    let shutdown = ShutdownBudget::new(
        Duration::from_secs(5),
        second,
        second,
        cleanup,
    )
    .map_err(|_| ApplicationFailure)?;
    let supervisor = Supervisor::new(shutdown);
    let admission = supervisor.operation_admission();
    let request_budget = ResponseConstructionBudget::new(OPERATION_BUDGET)
        .map_err(|_| ApplicationFailure)?;
    let startup_context = OperationContext::new(Duration::from_secs(15))
        .map_err(|_| ApplicationFailure)?;
    let (address_sender, address_receiver) = oneshot::channel();

    let mut starting = Startup::scoped(
        supervisor,
        startup_context,
        cleanup,
        move |scope| {
            Box::pin(async move {
                scope.stage("database.startup").map_err(|_| ApplicationFailure)?;
                let database_url = env::var("DATABASE_URL").map_err(|_| ApplicationFailure)?;
                let connect_options = PgConnectOptions::from_str(&database_url)
                    .map_err(|_| ApplicationFailure)?;
                let pool_slot = scope
                    .reserve_cleanup("database.close")
                    .map_err(|_| ApplicationFailure)?;
                let pool = pool_in(
                    pool_slot,
                    PgPoolOptions::new().max_connections(4),
                    connect_options,
                );

                let initial_query_context = OperationContext::new(OPERATION_BUDGET)
                    .map_err(|_| ApplicationFailure)?;
                if select_42(&pool, &initial_query_context).await? != 42 {
                    return Err(ApplicationFailure);
                }

                let (failure_sender, mut failure_receiver) = mpsc::channel(1);
                let worker_pool = pool.clone();
                scope
                    .registration()
                    .register("failure.worker", move |startup| async move {
                        let shutdown = startup.acknowledge_started();
                        tokio::select! {
                            biased;
                            _ = shutdown.draining() => Ok(()),
                            message = failure_receiver.recv() => {
                                match message {
                                    Some(()) => {
                                        let context = OperationContext::new(OPERATION_BUDGET)
                                            .map_err(|_| Box::new(ApplicationFailure) as BoxError)?;
                                        let _ = query_absent_relation(&worker_pool, &context).await;
                                        Err(Box::new(ApplicationFailure) as BoxError)
                                    }
                                    None => Ok(()),
                                }
                            }
                        }
                    })
                    .map_err(|_| ApplicationFailure)?;

                scope.stage("http.bind").map_err(|_| ApplicationFailure)?;
                let listener = TcpListener::bind("127.0.0.1:0")
                    .await
                    .map_err(|_| ApplicationFailure)?;
                let address = listener.local_addr().map_err(|_| ApplicationFailure)?;
                let state = ApplicationState {
                    pool,
                    failure: failure_sender,
                };
                let guarded = Router::new()
                    .route("/query", get(query))
                    .route("/trigger-failure", get(trigger_failure))
                    .route_layer(middleware::from_fn_with_state(
                        RequestPolicy::new(admission, request_budget),
                        request_admission,
                    ))
                    .with_state(state);
                register_http_in(scope, "http", listener, guarded)
                    .map_err(|_| ApplicationFailure)?;
                address_sender.send(address).map_err(|_| ApplicationFailure)?;
                Ok(())
            })
        },
    )
    .with_unix_signals("signals")
    .start();

    let mut running = starting.wait().await.map_err(|_| ApplicationFailure)?;
    running
        .status()
        .wait_ready()
        .await
        .map_err(|_| ApplicationFailure)?;
    let address = address_receiver.await.map_err(|_| ApplicationFailure)?;
    println!("consumer-listening:http://{address}");
    running.wait().await.map_err(|_| ApplicationFailure)
}

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(report) => {
            if report.cleanup.is_success() {
                println!("consumer-cleanup:database.close:succeeded");
            }
            if report.is_success() {
                println!("consumer-result:success");
                ExitCode::SUCCESS
            } else {
                println!("consumer-result:failure");
                eprintln!("consumer failed");
                ExitCode::FAILURE
            }
        }
        Err(_) => {
            println!("consumer-result:failure");
            eprintln!("consumer failed");
            ExitCode::FAILURE
        }
    }
}
