//! Test-owned protected composition for borrowed-waiter and owner loss.
//!
//! `runtime::run` does not expose its startup owner, so these children compose
//! the same validated native options, `pool_in` and startup-owned Unix signals
//! directly. They add no control API to the production root.
use super::{
    ProbeResult,
    startup_process::{self, Signal, StartupChild},
};
use batter::{
    BoxError,
    cleanup::{CleanupBudget, CleanupOutcome},
    lifecycle::{Readiness, ShutdownBudget},
    operation::OperationContext,
    settings::SettingsSource,
    startup::{
        InitializationError, StartingSupervisor, Startup, StartupCause, StartupError,
        StartupFailure, StartupOutcome,
    },
};
use batter_example_reference_service::config::ServingSettings;
use sqlx::PgPool;
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::{sync::oneshot, task::JoinHandle, time::timeout};

const STAGE: &str = "test.hold";
const WAITER_DROPPED: &str = "protected-waiter-dropped";
const OWNER_DROPPED: &str = "protected-owner-dropped";
const BOUND: Duration = Duration::from_secs(5);

type Failure = InitializationError<BoxError>;

struct Held {
    starting: StartingSupervisor<Failure>,
    published: oneshot::Receiver<PgPool>,
    dependent_saw_open_pool: Arc<AtomicBool>,
    readiness: JoinHandle<Result<(), Readiness>>,
}

fn start() -> Held {
    let settings = ServingSettings::from_process(None, SettingsSource::default())
        .expect("child settings are valid");
    let second = Duration::from_secs(1);
    let cleanup = CleanupBudget::new(second * 3, second * 3, second).unwrap();
    let supervisor =
        settings.supervisor(ShutdownBudget::new(second * 2, second, second, cleanup).unwrap());
    let handle = supervisor.handle();
    // Resolves Ok only if readiness is ever acknowledged.
    let readiness = tokio::spawn(async move { handle.wait_ready().await });
    let connection = settings
        .connect_options_from_process()
        .expect("validated native options");
    let pool_options = settings.pool_options();
    let (publish, published) = oneshot::channel();
    let dependent_saw_open_pool = Arc::new(AtomicBool::new(false));
    let witness = dependent_saw_open_pool.clone();
    let context = OperationContext::new(Duration::from_secs(8)).unwrap();
    let starting = Startup::scoped(supervisor, context, cleanup, move |scope| {
        Box::pin(async move {
            scope.stage("postgres.acquire")?;
            let slot = scope.reserve_cleanup("postgres.pool")?;
            let pool = batter_sqlx::pool_in(slot, pool_options, connection);
            let value: i32 = sqlx::query_scalar("SELECT 1").fetch_one(&pool).await?;
            if value != 1 {
                return Err("unexpected native query result".into());
            }
            let dependent = pool.clone();
            scope
                .reserve_cleanup("test.dependent")?
                .register(move || async move {
                    // LIFO requires this dependent to finish before pool close.
                    witness.store(!dependent.is_closed(), Ordering::SeqCst);
                    Ok(())
                });
            scope.stage(STAGE)?;
            let _ = publish.send(pool);
            std::future::pending::<()>().await;
            Ok::<_, BoxError>(())
        })
    })
    .with_unix_signals("signals")
    .start();
    Held {
        starting,
        published,
        dependent_saw_open_pool,
        readiness,
    }
}

fn failed(outcome: StartupOutcome<Failure>) -> Arc<StartupFailure<Failure>> {
    match outcome {
        StartupOutcome::Failed(StartupError::Failed(report)) => report,
        _ => panic!("startup did not retain a failed report"),
    }
}

async fn check(
    report: &StartupFailure<Failure>,
    pool: &PgPool,
    dependent_saw_open_pool: &AtomicBool,
    readiness: JoinHandle<Result<(), Readiness>>,
) {
    assert_eq!(report.stage, STAGE);
    assert!(matches!(report.cause, StartupCause::Draining));
    assert!(report.destruction_panic.is_none());
    assert!(report.cleanup.skipped.is_empty());
    let records: Vec<_> = report
        .cleanup
        .records
        .iter()
        .map(|record| (record.name, &record.outcome))
        .collect();
    assert_eq!(
        records,
        [
            ("test.dependent", &CleanupOutcome::Succeeded),
            ("postgres.pool", &CleanupOutcome::Succeeded),
        ]
    );
    assert!(dependent_saw_open_pool.load(Ordering::SeqCst));
    // Canonical local close evidence: closed, no accounted size, later PoolClosed.
    assert!(pool.is_closed());
    assert_eq!(pool.size(), 0);
    assert!(matches!(pool.acquire().await, Err(sqlx::Error::PoolClosed)));
    assert!(readiness.await.expect("readiness observer joined").is_err());
}

pub async fn waiter_loss_child() {
    let Held {
        mut starting,
        published,
        dependent_saw_open_pool,
        readiness,
    } = start();
    let observer = starting.observer();
    let pool = {
        let wait = starting.wait();
        tokio::pin!(wait);
        tokio::select! {
            biased;
            _ = &mut wait => panic!("startup completed before its held stage"),
            pool = timeout(BOUND, published) => pool
                .expect("initializer reached its held stage")
                .expect("initializer published its pool"),
        }
        // The borrowed waiter is destroyed here; the owner remains alive.
    };
    assert!(
        timeout(Duration::from_millis(100), observer.wait())
            .await
            .is_err(),
        "borrowed waiter loss completed startup"
    );
    startup_process::event(WAITER_DROPPED);
    let observed = failed(
        timeout(BOUND, observer.wait())
            .await
            .expect("drain completes"),
    );
    let resumed = match timeout(BOUND, starting.wait())
        .await
        .expect("owner resumes")
    {
        Err(StartupError::Failed(report)) => report,
        _ => panic!("resumed owner did not retain the failed startup"),
    };
    assert!(Arc::ptr_eq(&observed, &resumed));
    check(&observed, &pool, &dependent_saw_open_pool, readiness).await;
}

pub async fn owner_loss_child() {
    let Held {
        starting,
        published,
        dependent_saw_open_pool,
        readiness,
    } = start();
    let observer = starting.observer();
    let pool = timeout(BOUND, published)
        .await
        .expect("initializer reached its held stage")
        .expect("initializer published its pool");
    // Retain only the observer. Owner destruction requests drain without a signal.
    drop(starting);
    startup_process::event(OWNER_DROPPED);
    let report = failed(
        timeout(BOUND, observer.wait())
            .await
            .expect("drain completes"),
    );
    check(&report, &pool, &dependent_saw_open_pool, readiness).await;
}

pub async fn waiter_loss(pool: PgPool) -> ProbeResult {
    let endpoint = startup_process::endpoint(&pool);
    let mut child = StartupChild::start("protected.waiter-loss", &endpoint)?;
    let outcome = tokio::task::spawn_blocking(move || {
        child.wait_event(WAITER_DROPPED, BOUND)?;
        child.stop(
            Signal::Term,
            &[WAITER_DROPPED],
            &[&endpoint, "fixture-token"],
        )
    })
    .await?;
    outcome.map_err(Into::into)
}

pub async fn owner_loss(pool: PgPool) -> ProbeResult {
    let endpoint = startup_process::endpoint(&pool);
    let mut child = StartupChild::start("protected.owner-loss", &endpoint)?;
    let outcome = tokio::task::spawn_blocking(move || {
        child.wait_event(OWNER_DROPPED, BOUND)?;
        child.finish(&[OWNER_DROPPED], &[&endpoint, "fixture-token"])
    })
    .await?;
    outcome.map_err(Into::into)
}
