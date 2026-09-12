//! Ignored in ordinary gates; scripts/test_sqlx_live.sh requires every case.

use batter::{
    BoxError, RegistrationError,
    cleanup::{CleanupBudget, CleanupOutcome, CleanupReport},
    command::{Command, CommandCause},
    lifecycle::{ShutdownBudget, Supervisor},
    operation::{Interruption, OperationContext},
    startup::Startup,
};
use batter_sqlx::{SqlxFailure, pool_in, probe};
use sqlx::{
    PgPool,
    postgres::{PgConnectOptions, PgPoolOptions, PgSslMode},
};
use std::{
    str::FromStr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::oneshot;

type Result<T = ()> = std::result::Result<T, BoxError>;

fn cleanup_budget() -> CleanupBudget {
    CleanupBudget::new(
        Duration::from_secs(3),
        Duration::from_secs(2),
        Duration::from_secs(1),
    )
    .unwrap()
}

fn operation() -> OperationContext {
    OperationContext::new(Duration::from_secs(8)).unwrap()
}

fn options() -> PgPoolOptions {
    PgPoolOptions::new()
        .max_connections(3)
        .acquire_timeout(Duration::from_secs(2))
}

fn connection() -> PgConnectOptions {
    PgConnectOptions::from_str(
        &std::env::var("DATABASE_URL").expect("explicit live runner requires DATABASE_URL"),
    )
    .expect("runner supplied a native PostgreSQL URL")
}

fn supervisor() -> Supervisor {
    Supervisor::new(
        ShutdownBudget::new(
            Duration::from_secs(2),
            Duration::from_secs(1),
            Duration::from_secs(1),
            cleanup_budget(),
        )
        .unwrap(),
    )
}

async fn wait_until(mut predicate: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while !predicate() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("observable native transition timed out");
}

async fn assert_closed(pool: &PgPool, cleanup: &CleanupReport, names: &[&str]) {
    assert!(cleanup.is_success(), "{cleanup:?}");
    assert!(cleanup.skipped.is_empty());
    assert_eq!(
        cleanup
            .records
            .iter()
            .map(|record| record.name)
            .collect::<Vec<_>>(),
        names
    );
    assert_eq!(pool.size(), 0);
    assert!(pool.is_closed());
    assert!(matches!(pool.acquire().await, Err(sqlx::Error::PoolClosed)));
}

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn command_query_closes_owned_pool() -> Result {
    let observed = Arc::new(Mutex::new(None));
    let published = observed.clone();
    let report = Command::new(operation(), cleanup_budget(), move |scope| {
        Box::pin(async move {
            let pool = pool_in(
                scope.reserve_cleanup("postgres.pool")?,
                options(),
                connection(),
            );
            *published.lock().unwrap() = Some(pool.clone());
            let answer: i32 = sqlx::query_scalar("SELECT 1::int4")
                .fetch_one(&pool)
                .await?;
            Ok::<_, BoxError>(answer)
        })
    })
    .start()
    .wait()
    .await
    .unwrap();
    assert_eq!(report.work.as_ref().unwrap(), &1);
    let pool = observed.lock().unwrap().clone().unwrap();
    assert_closed(&pool, report.cleanup.as_ref().unwrap(), &["postgres.pool"]).await;
    Ok(())
}

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn startup_query_joins_before_pool_close() -> Result {
    let events = Arc::new(Mutex::new(Vec::new()));
    let observed = Arc::new(Mutex::new(None));
    let published = observed.clone();
    let stopped = events.clone();
    let finalized = events.clone();
    let mut starting = Startup::new(supervisor(), operation(), cleanup_budget(), move |scope| {
        Box::pin(async move {
            let pool = pool_in(
                scope.supervisor().reserve_cleanup("postgres.pool")?,
                options(),
                connection(),
            );
            *published.lock().unwrap() = Some(pool.clone());
            let answer: i32 = sqlx::query_scalar("SELECT 1::int4")
                .fetch_one(&pool)
                .await?;
            assert_eq!(answer, 1);
            scope
                .supervisor()
                .reserve_cleanup("dependent")?
                .register(move || async move {
                    finalized.lock().unwrap().push("dependent");
                    Ok(())
                });
            scope
                .supervisor()
                .register("worker", move |shutdown| async move {
                    shutdown.mark_started();
                    shutdown.draining().await;
                    stopped.lock().unwrap().push("worker");
                    Ok(())
                })?;
            Ok::<_, BoxError>(())
        })
    })
    .start();
    let running = starting.wait().await.unwrap();
    running.handle().wait_ready().await.unwrap();
    running.handle().request();
    let report = running.wait().await.unwrap();
    assert!(report.is_success(), "{report:?}");
    assert_eq!(*events.lock().unwrap(), ["worker", "dependent"]);
    let pool = observed.lock().unwrap().clone().unwrap();
    assert_closed(&pool, &report.cleanup, &["dependent", "postgres.pool"]).await;
    Ok(())
}

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn invalid_slot_prevents_pool_construction() -> Result {
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = calls.clone();
    let report = Command::new(operation(), cleanup_budget(), move |scope| {
        Box::pin(async move {
            assert!(scope.reserve_cleanup("").is_err());
            if let Ok(slot) = scope.reserve_cleanup("") {
                counted.fetch_add(1, Ordering::SeqCst);
                drop(pool_in(slot, options(), connection()));
            }
            Ok::<_, RegistrationError>(())
        })
    })
    .start()
    .wait()
    .await
    .unwrap();
    assert!(report.is_success());
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(report.cleanup.as_ref().unwrap().records.is_empty());
    Ok(())
}

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn duplicate_slot_prevents_second_pool() -> Result {
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = calls.clone();
    let observed = Arc::new(Mutex::new(None));
    let published = observed.clone();
    let report = Command::new(operation(), cleanup_budget(), move |scope| {
        Box::pin(async move {
            let first = pool_in(scope.reserve_cleanup("pool")?, options(), connection());
            *published.lock().unwrap() = Some(first);
            assert!(scope.reserve_cleanup("pool").is_err());
            if let Ok(slot) = scope.reserve_cleanup("pool") {
                counted.fetch_add(1, Ordering::SeqCst);
                drop(pool_in(slot, options(), connection()));
            }
            Ok::<_, RegistrationError>(())
        })
    })
    .start()
    .wait()
    .await
    .unwrap();
    assert!(report.is_success());
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let pool = observed.lock().unwrap().clone().unwrap();
    assert_closed(&pool, report.cleanup.as_ref().unwrap(), &["pool"]).await;
    Ok(())
}

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn native_options_and_maintenance_are_preserved() -> Result {
    let connects = Arc::new(AtomicUsize::new(0));
    let counted = connects.clone();
    let inside = connects.clone();
    let report = Command::new(operation(), cleanup_budget(), move |scope| {
        Box::pin(async move {
            let native = options()
                .max_connections(4)
                .min_connections(1)
                .idle_timeout(None)
                .max_lifetime(None)
                .after_connect(move |connection, _| {
                    counted.fetch_add(1, Ordering::SeqCst);
                    Box::pin(async move {
                        sqlx::query("SET application_name = 'batter_pool_ownership'")
                            .execute(connection)
                            .await?;
                        Ok(())
                    })
                });
            let pool = pool_in(scope.reserve_cleanup("pool")?, native, connection());
            assert_eq!(pool.options().get_max_connections(), 4);
            assert_eq!(pool.options().get_min_connections(), 1);
            wait_until(|| inside.load(Ordering::SeqCst) >= 1).await;
            let name: String = sqlx::query_scalar("SHOW application_name")
                .fetch_one(&pool)
                .await?;
            assert_eq!(name, "batter_pool_ownership");
            Ok::<_, BoxError>(())
        })
    })
    .start()
    .wait()
    .await
    .unwrap();
    assert!(report.is_success(), "{report:?}");
    assert!(connects.load(Ordering::SeqCst) >= 1);
    Ok(())
}

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn authentication_error_keeps_pool_cleanup() -> Result {
    let observed = Arc::new(Mutex::new(None));
    let published = observed.clone();
    let report = Command::new(operation(), cleanup_budget(), move |scope| {
        Box::pin(async move {
            let rejected = connection()
                .username("missing_pool_ownership_role")
                .password("invalid-pool-ownership-password");
            let pool = pool_in(scope.reserve_cleanup("pool").unwrap(), options(), rejected);
            *published.lock().unwrap() = Some(pool.clone());
            probe(&pool, &operation()).await
        })
    })
    .start()
    .wait()
    .await
    .unwrap();
    assert!(matches!(
        &report.work,
        Err(CommandCause::Failed(batter::operation::OperationError::Failed(error)))
            if matches!(error.native(), sqlx::Error::Database(database)
                if matches!(database.code().as_deref(), Some("28P01" | "28000")))
    ));
    let pool = observed.lock().unwrap().clone().unwrap();
    assert_closed(&pool, report.cleanup.as_ref().unwrap(), &["pool"]).await;
    Ok(())
}

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn cancelled_acquisition_keeps_pool_cleanup() -> Result {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    let observed = Arc::new(Mutex::new(None));
    let published = observed.clone();
    let command = Command::new(operation(), cleanup_budget(), move |scope| {
        Box::pin(async move {
            let target = PgConnectOptions::new_without_pgpass()
                .host("127.0.0.1")
                .port(port)
                .username("unused")
                .database("unused")
                .ssl_mode(PgSslMode::Disable);
            let pool = pool_in(
                scope.reserve_cleanup("pool")?,
                options().acquire_timeout(Duration::from_secs(20)),
                target,
            );
            *published.lock().unwrap() = Some(pool.clone());
            pool.acquire().await?;
            Ok::<_, BoxError>(())
        })
    })
    .start();
    let (socket, _) = tokio::time::timeout(Duration::from_secs(2), listener.accept()).await??;
    let observer = command.observer();
    drop(command);
    drop(socket);
    drop(listener);
    let report = observer.wait().await.unwrap();
    assert!(matches!(
        report.work,
        Err(CommandCause::Interrupted(Interruption::Cancelled))
    ));
    let pool = observed.lock().unwrap().clone().unwrap();
    assert_closed(&pool, report.cleanup.as_ref().unwrap(), &["pool"]).await;
    Ok(())
}

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn later_application_error_keeps_pool_cleanup() -> Result {
    let observed = Arc::new(Mutex::new(None));
    let published = observed.clone();
    let report = Command::new(operation(), cleanup_budget(), move |scope| {
        Box::pin(async move {
            let pool = pool_in(scope.reserve_cleanup("pool")?, options(), connection());
            *published.lock().unwrap() = Some(pool.clone());
            let _: i32 = sqlx::query_scalar("SELECT 1::int4")
                .fetch_one(&pool)
                .await?;
            Err::<(), BoxError>(std::io::Error::other("private-work-marker").into())
        })
    })
    .start()
    .wait()
    .await
    .unwrap();
    assert!(matches!(&report.work, Err(CommandCause::Failed(error))
        if error.downcast_ref::<std::io::Error>().is_some_and(|error| error.to_string() == "private-work-marker")));
    let pool = observed.lock().unwrap().clone().unwrap();
    assert_closed(&pool, report.cleanup.as_ref().unwrap(), &["pool"]).await;
    assert!(!format!("{report:?}").contains("private-work-marker"));
    Ok(())
}

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn application_panic_keeps_pool_cleanup() -> Result {
    let observed = Arc::new(Mutex::new(None));
    let published = observed.clone();
    let report = Command::new(operation(), cleanup_budget(), move |scope| {
        Box::pin(async move {
            let pool = pool_in(scope.reserve_cleanup("pool")?, options(), connection());
            *published.lock().unwrap() = Some(pool.clone());
            let _: i32 = sqlx::query_scalar("SELECT 1::int4")
                .fetch_one(&pool)
                .await
                .unwrap();
            panic!("pool-application-panic-marker");
            #[allow(unreachable_code)]
            Ok::<(), RegistrationError>(())
        })
    })
    .start()
    .wait()
    .await
    .unwrap();
    let Err(CommandCause::Panicked(payload)) = &report.work else {
        panic!("application panic was not retained")
    };
    payload
        .try_inspect(|payload| {
            assert_eq!(
                payload.downcast_ref::<&str>(),
                Some(&"pool-application-panic-marker")
            );
        })
        .unwrap();
    let pool = observed.lock().unwrap().clone().unwrap();
    assert_closed(&pool, report.cleanup.as_ref().unwrap(), &["pool"]).await;
    Ok(())
}

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn two_pools_close_after_dependents_in_lifo_order() -> Result {
    let observed = Arc::new(Mutex::new(None));
    let published = observed.clone();
    let report = Command::new(operation(), cleanup_budget(), move |scope| {
        Box::pin(async move {
            let first = pool_in(scope.reserve_cleanup("first")?, options(), connection());
            let second = pool_in(scope.reserve_cleanup("second")?, options(), connection());
            let first_check = first.clone();
            let second_check = second.clone();
            sqlx::query("SELECT 1").execute(&first).await?;
            sqlx::query("SELECT 1").execute(&second).await?;
            scope
                .reserve_cleanup("dependent")?
                .register(move || async move {
                    assert!(!first_check.is_closed());
                    assert!(!second_check.is_closed());
                    Ok(())
                });
            *published.lock().unwrap() = Some((first, second));
            Ok::<_, BoxError>(())
        })
    })
    .start()
    .wait()
    .await
    .unwrap();
    assert!(report.is_success(), "{report:?}");
    assert_eq!(
        report
            .cleanup
            .as_ref()
            .unwrap()
            .records
            .iter()
            .map(|record| record.name)
            .collect::<Vec<_>>(),
        ["dependent", "second", "first"]
    );
    let (first, second) = observed.lock().unwrap().clone().unwrap();
    assert!(first.is_closed() && second.is_closed());
    Ok(())
}

async fn held_checkout(release_before_timeout: bool) -> Result {
    let budget = if release_before_timeout {
        cleanup_budget()
    } else {
        CleanupBudget::new(
            Duration::from_millis(100),
            Duration::from_millis(50),
            Duration::from_secs(1),
        )?
    };
    let observed = Arc::new(Mutex::new(None));
    let published = observed.clone();
    let (held_tx, held_rx) = oneshot::channel();
    let command = Command::new(operation(), budget, move |scope| {
        Box::pin(async move {
            let pool = pool_in(scope.reserve_cleanup("pool")?, options(), connection());
            *published.lock().unwrap() = Some(pool.clone());
            held_tx.send(pool.acquire().await?).unwrap();
            Ok::<_, BoxError>(())
        })
    })
    .start();
    let held = held_rx.await?;
    let pool = observed.lock().unwrap().clone().unwrap();
    let wait = tokio::spawn(async move { command.wait().await });
    wait_until(|| pool.is_closed()).await;
    if release_before_timeout {
        drop(held);
        let report = wait.await??;
        assert_closed(&pool, report.cleanup.as_ref().unwrap(), &["pool"]).await;
    } else {
        let report = wait.await??;
        let cleanup = report.cleanup.as_ref().unwrap();
        assert!(!report.is_success());
        assert_eq!(cleanup.records[0].name, "pool");
        assert_eq!(cleanup.records[0].outcome, CleanupOutcome::TimedOut);
        drop(held);
        tokio::time::timeout(Duration::from_secs(2), pool.close()).await?;
    }
    Ok(())
}

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn held_checkout_delays_successful_close() -> Result {
    held_checkout(true).await
}

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn held_checkout_timeout_is_not_success() -> Result {
    held_checkout(false).await
}

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn work_and_cleanup_failures_are_both_retained() -> Result {
    let observed = Arc::new(Mutex::new(None));
    let published = observed.clone();
    let report = Command::new(operation(), cleanup_budget(), move |scope| {
        Box::pin(async move {
            scope.reserve_cleanup("failed").unwrap().register(|| async {
                Err::<(), BoxError>(std::io::Error::other("private-cleanup-marker").into())
            });
            let pool = pool_in(
                scope.reserve_cleanup("pool").unwrap(),
                options(),
                connection(),
            );
            *published.lock().unwrap() = Some(pool.clone());
            let error = sqlx::query("SELECT 1 / 0")
                .execute(&pool)
                .await
                .unwrap_err();
            Err::<(), SqlxFailure>(error.into())
        })
    })
    .start()
    .wait()
    .await
    .unwrap();
    assert!(matches!(&report.work, Err(CommandCause::Failed(error))
        if matches!(error.native(), sqlx::Error::Database(database) if database.code().as_deref() == Some("22012"))));
    let cleanup = report.cleanup.as_ref().unwrap();
    assert_eq!(
        cleanup
            .records
            .iter()
            .map(|record| (record.name, record.outcome))
            .collect::<Vec<_>>(),
        [
            ("pool", CleanupOutcome::Succeeded),
            ("failed", CleanupOutcome::Failed)
        ]
    );
    let pool = observed.lock().unwrap().clone().unwrap();
    assert!(pool.is_closed());
    assert!(!format!("{report:?}").contains("private-cleanup-marker"));
    Ok(())
}

fn cleanup_oracle(cleanup: &CleanupReport, pool: &PgPool, close_finished: bool) -> Result {
    if cleanup.records.len() != 1
        || cleanup.records[0].name != "pool"
        || cleanup.records[0].outcome != CleanupOutcome::Succeeded
        || !cleanup.skipped.is_empty()
        || !pool.is_closed()
        || !close_finished
    {
        return Err(std::io::Error::other("pool cleanup was not completely observed").into());
    }
    Ok(())
}

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn ownership_oracles_reject_missing_and_premature_cleanup() -> Result {
    let missing = options().connect_lazy_with(connection());
    sqlx::query("SELECT 1").execute(&missing).await?;
    assert!(cleanup_oracle(&CleanupReport::default(), &missing, false).is_err());
    missing.close().await;

    let premature = options().connect_lazy_with(connection());
    let held = premature.acquire().await?;
    let closing = premature.clone();
    let close = tokio::spawn(async move { closing.close().await });
    wait_until(|| premature.is_closed()).await;
    let claimed = CleanupReport::default();
    assert!(cleanup_oracle(&claimed, &premature, close.is_finished()).is_err());
    drop(held);
    tokio::time::timeout(Duration::from_secs(2), close).await??;
    Ok(())
}
