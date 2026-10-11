//! Offline contracts for PostgreSQL limiters with registered expiry cleanup.
//!
//! No database is contacted. A closed pool fails acquisition at once, and a
//! local listener that accepts connections but never answers holds an
//! acquisition until a native timeout. Live cleanup under restricted grants is
//! in the facade's `grants_live` suite.

use batter_core::{
    RegistrationError,
    cleanup::CleanupBudget,
    lifecycle::{Readiness, ShutdownBudget, ShutdownCause, Supervisor},
    operation::OperationOwner,
    periodic::{
        PeriodicCompletion, PeriodicInitializationExpired, PeriodicReader, PeriodicShutdown,
        PeriodicStartup,
    },
};
use batter_runlimit::{
    Checks, Quota, RunResult,
    maintenance::{ExpiryMaintenance, MaintainedLimiter, register_in},
};
use runlimit_core::{
    Check, ConsumptionStatus, FixedWindowPolicy, KeyHasher, Observation, Observer, PolicyId,
    ScopeId,
};
use runlimit_postgres::{
    CleanupPhase, MaintenanceError, PostgresConfig, PostgresGcraLimiter, PostgresLimiter,
};
use sqlx::{
    PgPool,
    postgres::{PgConnectOptions, PgPoolOptions, PgSslMode},
};
use std::{
    convert::Infallible,
    num::NonZeroU32,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{net::TcpListener, task::JoinHandle, time::Instant};

const ROWS: NonZeroU32 = NonZeroU32::new(64).unwrap();

fn supervisor() -> Supervisor {
    let second = Duration::from_secs(1);
    Supervisor::new(
        ShutdownBudget::new(
            second,
            second,
            second,
            CleanupBudget::new(second, second, second).unwrap(),
        )
        .unwrap(),
    )
}

fn maintenance(interval: Duration, startup: PeriodicStartup) -> ExpiryMaintenance {
    ExpiryMaintenance::new(interval, ROWS, startup, PeriodicShutdown::StopAtDrain).unwrap()
}

fn local_options(port: u16) -> PgConnectOptions {
    PgConnectOptions::new_without_pgpass()
        .host("127.0.0.1")
        .port(port)
        .username("maintenance")
        .database("maintenance")
        .ssl_mode(PgSslMode::Disable)
}

/// A pool whose every acquisition fails at once with `PoolClosed`.
async fn closed_pool() -> PgPool {
    let pool = PgPoolOptions::new().connect_lazy_with(local_options(1));
    pool.close().await;
    pool
}

/// A pool whose acquisitions hang: the listener accepts and never answers.
/// SQLx's own acquisition timeout is far longer than any native budget used
/// here, so only native and periodic deadlines can end an acquisition.
async fn silent_pool() -> (PgPool, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let accepting = tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((socket, _)) = listener.accept().await {
            held.push(socket);
        }
    });
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(Duration::from_secs(300))
        .connect_lazy_with(local_options(port));
    (pool, accepting)
}

fn native_config(acquire: Duration, operation: Duration) -> PostgresConfig {
    PostgresConfig::new()
        .with_pool_acquire_timeout(acquire)
        .unwrap()
        .with_operation_timeout(operation)
        .unwrap()
}

#[derive(Default)]
struct CleanupObservations(AtomicUsize);

impl Observer for CleanupObservations {
    fn observe(&self, observation: &Observation<'_>) {
        if matches!(observation, Observation::Cleanup(_)) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
}

fn retained_cause(reader: &PeriodicReader) -> Arc<dyn std::error::Error + Send + Sync> {
    reader
        .snapshot()
        .first_failure
        .expect("a failed cleanup is retained")
        .error
}

async fn wait_for(reader: &PeriodicReader, done: impl Fn(&PeriodicReader) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done(reader) {
        assert!(Instant::now() < deadline, "{:?}", reader.snapshot());
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

#[tokio::test(start_paused = true)]
async fn failed_cleanups_are_retained_and_recur_without_draining() {
    let mut supervisor = supervisor();
    let fixed = register_in(
        &mut supervisor,
        "quota.fixed.maintenance",
        PostgresLimiter::new(closed_pool().await),
        maintenance(Duration::from_secs(1), PeriodicStartup::immediate()),
    )
    .unwrap();
    let gcra = register_in(
        &mut supervisor,
        "quota.gcra.maintenance",
        PostgresGcraLimiter::new(closed_pool().await),
        maintenance(Duration::from_secs(1), PeriodicStartup::immediate()),
    )
    .unwrap();
    let running = supervisor.start();
    running.status().wait_ready().await.unwrap();
    tokio::time::sleep(Duration::from_millis(2_500)).await;

    for reader in [fixed.maintenance(), gcra.maintenance()] {
        let snapshot = reader.snapshot();
        // Runs at 0 s, 1 s and 2 s each fail; none drains the process.
        assert_eq!(snapshot.invocations, 3, "{snapshot:?}");
        assert_eq!(snapshot.recoverable_failures, 3, "{snapshot:?}");
        assert_eq!(snapshot.deadline_exceeded, 0, "{snapshot:?}");
        assert!(matches!(
            retained_cause(reader).downcast_ref::<MaintenanceError>(),
            Some(MaintenanceError::Database(sqlx::Error::PoolClosed)),
        ));
    }
    assert_eq!(running.status().readiness(), Readiness::Ready);

    let report = running.shutdown().await.unwrap();
    assert_eq!(report.cause, ShutdownCause::Requested);
    assert_eq!(report.periodic.len(), 2);
    for record in &report.periodic {
        assert_eq!(record.summary.completion, PeriodicCompletion::Stopped);
        assert!(record.summary.acknowledged);
        assert!(record.summary.terminal_failure.is_none());
    }
}

#[tokio::test(start_paused = true)]
async fn first_success_startup_never_publishes_readiness_for_failing_cleanup() {
    let mut supervisor = supervisor();
    let limiter = register_in(
        &mut supervisor,
        "quota.maintenance",
        PostgresLimiter::new(closed_pool().await),
        maintenance(
            Duration::from_secs(1),
            PeriodicStartup::after_first_success(Duration::from_millis(2_500)).unwrap(),
        ),
    )
    .unwrap();
    let running = supervisor.start();
    assert!(running.status().wait_ready().await.is_err());

    let report = running.wait_report().await.unwrap();
    assert!(!report.is_success());
    let summary = &report.periodic[0].summary;
    assert_eq!(
        summary.completion,
        PeriodicCompletion::InitializationExpired
    );
    assert!(!summary.acknowledged);
    assert_eq!(summary.recoverable_failures, 3, "{summary:?}");
    let terminal = summary.terminal_failure.as_ref().unwrap();
    assert!(
        terminal
            .error
            .downcast_ref::<PeriodicInitializationExpired>()
            .is_some()
    );
    assert!(matches!(
        retained_cause(limiter.maintenance()).downcast_ref::<MaintenanceError>(),
        Some(MaintenanceError::Database(sqlx::Error::PoolClosed)),
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_timeouts_classify_a_hung_acquisition_before_the_run_deadline() {
    let (pool, accepting) = silent_pool().await;
    let millis = Duration::from_millis(200);
    let mut supervisor = supervisor();
    let limiter = register_in(
        &mut supervisor,
        "quota.maintenance",
        PostgresLimiter::new(pool).with_config(native_config(millis, millis)),
        maintenance(Duration::from_secs(60), PeriodicStartup::immediate()),
    )
    .unwrap();
    let running = supervisor.start();
    let reader = limiter.maintenance();
    wait_for(reader, |reader| {
        let snapshot = reader.snapshot();
        snapshot.recoverable_failures + snapshot.deadline_exceeded > 0
    })
    .await;

    // The derived run budget outlasts both native bounds, so the native
    // acquisition timeout reports the run before the periodic deadline can
    // destroy it and discard the classification.
    let snapshot = reader.snapshot();
    assert_eq!(snapshot.deadline_exceeded, 0, "{snapshot:?}");
    assert_eq!(snapshot.recoverable_failures, 1, "{snapshot:?}");
    assert!(matches!(
        retained_cause(reader).downcast_ref::<MaintenanceError>(),
        Some(MaintenanceError::TimedOutBeforeCommit {
            phase: CleanupPhase::AcquiringConnection,
        }),
    ));
    // Retained maintenance failures do not make the process report unsuccessful.
    assert!(running.shutdown().await.unwrap().is_success());
    accepting.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn drain_destroys_an_active_cleanup_before_finalizers_run() {
    let (pool, accepting) = silent_pool().await;
    let long = Duration::from_secs(30);
    let mut supervisor = supervisor();
    let limiter = register_in(
        &mut supervisor,
        "quota.maintenance",
        PostgresLimiter::new(pool.clone()).with_config(native_config(long, long)),
        maintenance(Duration::from_secs(60), PeriodicStartup::immediate()),
    )
    .unwrap();
    // Finalizers run in reverse registration order, so this probe runs before
    // the pool finalizer registered below it would in an application.
    let seen = Arc::new(Mutex::new(None));
    let probe = (limiter.maintenance().clone(), pool.clone(), seen.clone());
    supervisor
        .on_cleanup("probe.pool.close", move || async move {
            let (reader, pool, seen) = probe;
            *seen.lock().unwrap() = Some((reader.snapshot().completion, pool.is_closed()));
            pool.close().await;
            Ok(())
        })
        .unwrap();
    let running = supervisor.start();
    let reader = limiter.maintenance();
    wait_for(reader, |reader| reader.snapshot().invocations == 1).await;

    let started = Instant::now();
    let report = running.shutdown().await.unwrap();
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "drain waited for the native acquisition budget"
    );
    let summary = &report.periodic[0].summary;
    assert_eq!(summary.completion, PeriodicCompletion::Stopped);
    assert_eq!(summary.stop_interrupted, 1, "{summary:?}");
    assert_eq!(summary.recoverable_failures, 0, "{summary:?}");
    assert!(!report.forced_cancellation);
    assert!(report.cleanup.is_success());
    assert_eq!(
        *seen.lock().unwrap(),
        Some((PeriodicCompletion::Stopped, false)),
        "maintenance had not stopped, or the pool was already closed, when finalizers began",
    );
    assert!(pool.is_closed());
    accepting.abort();
}

#[tokio::test]
async fn registration_is_inert_and_validates_its_name() {
    let observations = Arc::new(CleanupObservations::default());
    let pool = closed_pool().await;
    let limiter = || PostgresLimiter::new(pool.clone()).with_observer(observations.clone());
    let schedule = maintenance(Duration::from_secs(1), PeriodicStartup::immediate());

    let mut supervisor = supervisor();
    assert!(matches!(
        register_in(&mut supervisor, "", limiter(), schedule),
        Err(RegistrationError::InvalidName),
    ));
    register_in(&mut supervisor, "quota.maintenance", limiter(), schedule).unwrap();
    assert!(matches!(
        register_in(&mut supervisor, "quota.maintenance", limiter(), schedule),
        Err(RegistrationError::Duplicate("quota.maintenance")),
    ));
    // An unstarted supervisor never runs registered cleanup.
    drop(supervisor);
    tokio::task::yield_now().await;
    assert_eq!(observations.0.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn the_maintained_limiter_checks_through_the_native_limiter() {
    let mut supervisor = supervisor();
    let limiter: MaintainedLimiter<PostgresLimiter> = register_in(
        &mut supervisor,
        "quota.maintenance",
        PostgresLimiter::new(closed_pool().await),
        maintenance(Duration::from_secs(1), PeriodicStartup::immediate()),
    )
    .unwrap();
    let policy = FixedWindowPolicy::new(
        PolicyId::new("maintenance.quota").unwrap(),
        ScopeId::new("owner").unwrap(),
        5,
        Duration::from_secs(60),
    )
    .unwrap();
    let checks = [Check::new(
        KeyHasher::new([3; 32])
            .unwrap()
            .hash_for(&policy, "owner-a"),
    )];
    let context = OperationOwner::new(Duration::from_secs(5))
        .unwrap()
        .into_context();
    let mut invoked = false;
    let result = Quota::new(limiter)
        .run(&context, Checks::new(&checks).unwrap(), |_| {
            invoked = true;
            async { Ok::<_, Infallible>(()) }
        })
        .await;
    assert!(matches!(
        result,
        RunResult::Backend {
            consumption: ConsumptionStatus::NotConsumed,
            ..
        }
    ));
    assert!(!invoked);
}
