//! The serving process: one protected startup that owns the pool, the flags
//! control plane, the receipt worker and the HTTP listener.

use std::{net::SocketAddr, time::Duration};

use batter::{
    BoxError,
    cleanup::CleanupBudget,
    flags::{
        control_plane::{
            ApplicationName, BuildId, ControlPlaneStore, HeartbeatCadence, ListenEndpoint,
            LoadPolicy, Presence, RefreshTiming, SnapshotDirectory, SnapshotFilePath, register_in,
        },
        declared::Resolved,
        postgres::StoreTiming,
    },
    lifecycle::{ShutdownBudget, Supervisor},
    operation::OperationOwner,
    runledger::{
        PgSessionProfile, RunledgerDatabase,
        native::runtime::{Supervisor as NativeSupervisor, config::JobsConfig},
        verify_schema,
    },
    startup::{ApplicationError, Startup},
};
use sqlx::postgres::PgPoolOptions;

use crate::{config::ServeConfig, flags::Handles, http, receipts};

fn budgets() -> Result<(ShutdownBudget, CleanupBudget), BoxError> {
    let second = Duration::from_secs(1);
    let cleanup = CleanupBudget::new(second * 5, second * 3, second)?;
    let shutdown = ShutdownBudget::new(second * 10, second * 5, second, cleanup)?;
    Ok((shutdown, cleanup))
}

/// Runs until SIGINT/SIGTERM or a fatal component exit, then reports the
/// checked shutdown.
pub async fn run(
    config: ServeConfig,
    resolved: Resolved,
    handles: Handles,
) -> Result<(), BoxError> {
    let (shutdown, cleanup) = budgets()?;
    let supervisor = Supervisor::new(shutdown);
    let lifecycle = supervisor.status();
    let admission = supervisor.operation_admission();
    let startup = Startup::protected(
        supervisor,
        OperationOwner::new(Duration::from_secs(60))?.into_context(),
        cleanup,
        async move |scope| -> Result<SocketAddr, ApplicationError> {
            let second = Duration::from_secs(1);
            let ServeConfig {
                database: login,
                last_good_dir,
                bind,
                request_budget,
                worker_id,
            } = config;

            // The pool's close is registered first, so every later finalizer,
            // the flags stop marker included, runs before it.
            scope.stage("database")?;
            let profile = PgSessionProfile::new(
                &login.role,
                &login.role,
                vec!["public".into()],
                Duration::ZERO,
                Duration::ZERO,
            )?;
            let pool = batter::sqlx::pool_in(
                scope.reserve_cleanup("database")?,
                // Application peak plus the three flags sessions (refresh,
                // heartbeat, prune) and the worker's loops.
                PgPoolOptions::new()
                    .max_connections(12)
                    .acquire_timeout(second * 5),
                login.options.clone(),
                profile,
            )?;
            let database = RunledgerDatabase::from_profiled(pool.clone())?;
            // Serving never migrates; refuse to start against an old schema.
            let _snapshot = verify_schema(&database).await?;

            scope.stage("flags")?;
            let store = ControlPlaneStore::new(
                &pool,
                StoreTiming::new(second * 2, second * 2, second, second * 2)?,
            );
            let last_good = SnapshotDirectory::open(SnapshotFilePath::new(
                last_good_dir,
                "checkout-service.bfsn",
            )?)?;
            let policy =
                LoadPolicy::start_on_defaults(RefreshTiming::new(second * 5, second * 60)?)
                    .with_last_good(last_good)
                    .with_listen(ListenEndpoint::new(login.options, second)?);
            let presence = Presence::new(
                ApplicationName::new("checkout-service")?,
                BuildId::new(env!("CARGO_PKG_VERSION"))?,
                HeartbeatCadence::new(second * 5, second * 600)?,
            );
            let flags = register_in(&mut *scope, store, resolved, policy, presence)?;

            scope.stage("receipts")?;
            let catalog = receipts::catalog(&flags, &handles, database.pool().clone())?;
            catalog.sync_definitions(database.pool()).await?;
            let jobs = JobsConfig {
                worker_id,
                poll_interval: Duration::from_millis(500),
                claim_batch_size: 10,
                lease_ttl_seconds: 60,
                max_global_concurrency: 4,
                reaper_interval: second * 15,
                schedule_poll_interval: second * 5,
                reaper_retry_delay_ms: 5_000,
            };
            let prepared = NativeSupervisor::builder(database.pool(), jobs)?
                .with_catalog(&catalog)
                .prepare()?;
            batter::runledger::register_in(
                &mut *scope,
                "receipts.worker",
                OperationOwner::new(second * 10)?.into_context(),
                prepared,
            )?;

            scope.stage("http")?;
            let gate = flags
                .http_gate(&handles.checkout_halt)?
                .with_disabled_level(tracing::Level::WARN);
            let state = http::AppState {
                database,
                express_receipt: handles.express_receipt.clone(),
            };
            let assembled =
                http::assemble(lifecycle, admission, request_budget, gate, state).await?;
            let listener = tokio::net::TcpListener::bind(bind).await?;
            let address = listener.local_addr()?;
            assembled.register_in(&mut *scope, "http", listener)?;
            Ok(address)
        },
    )
    .with_unix_signals("signals");
    let (running, address) = startup.start().finish_with_output().await?;
    println!("checkout-service: listening on {address}");
    running.wait_checked().await?;
    Ok(())
}
