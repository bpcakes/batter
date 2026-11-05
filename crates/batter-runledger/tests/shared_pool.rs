//! Protected acquisition and one shared cleanup owner; no PostgreSQL required.
#![allow(clippy::unwrap_used, reason = "a panic fails the test")]
#![allow(clippy::missing_assert_message, reason = "test names locate failures")]
use batter_core::{
    cleanup::{CleanupBudget, CleanupStack},
    lifecycle::{ShutdownBudget, Supervisor},
    operation::OperationOwner,
    startup::{ApplicationError, Startup, StartupCause, StartupError, StartupOutcome},
};
use batter_runledger::RunledgerDatabase;
use batter_sqlx::{PgSessionProfile, pool_in};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::sync::oneshot;

const BOUND: Duration = Duration::from_secs(3);
fn cleanup() -> CleanupBudget {
    CleanupBudget::new(BOUND, BOUND, BOUND).unwrap()
}
fn process() -> Supervisor {
    Supervisor::new(ShutdownBudget::new(BOUND, BOUND, BOUND, cleanup()).unwrap())
}
fn profile(schemas: &[&str]) -> PgSessionProfile {
    PgSessionProfile::new(
        "unused",
        "unused",
        schemas.iter().map(|s| (*s).into()).collect(),
        Duration::ZERO,
        Duration::ZERO,
    )
    .unwrap()
}
fn connection() -> PgConnectOptions {
    PgConnectOptions::new_without_pgpass()
        .host("127.0.0.1")
        .port(1)
        .username("unused")
        .database("unused")
}

#[tokio::test]
async fn one_finalizer_closes_every_view_after_dependent_drain() {
    let stopped = Arc::new(AtomicBool::new(false));
    let witness = stopped.clone();
    let starting = Startup::protected(
        process(),
        OperationOwner::new(BOUND).unwrap().into_context(),
        cleanup(),
        async move |scope| {
            let shared = pool_in(
                scope.reserve_cleanup("pool")?,
                PgPoolOptions::new(),
                connection(),
                profile(&["public"]),
            )?;
            let database = RunledgerDatabase::from_profiled(shared.clone())?;
            let dependent = database.clone();
            scope
                .registration()
                .register("dependent", move |startup| async move {
                    let shutdown = startup.acknowledge_started();
                    shutdown.draining().await;
                    assert!(
                        !dependent.pool().is_closed(),
                        "pool must outlive dependent work"
                    );
                    witness.store(true, Ordering::SeqCst);
                    Ok(shutdown.stopped())
                })?;
            Ok::<_, ApplicationError>((shared, database))
        },
    )
    .start();
    let (running, (shared, database)) = starting.finish_with_output().await.unwrap();
    running.status().wait_ready().await.unwrap();
    assert!(std::ptr::eq(shared.profile(), database.profile()));
    let report = running.shutdown().await.unwrap();
    assert!(report.is_success());
    assert!(stopped.load(Ordering::SeqCst));
    assert_eq!(report.cleanup.records.len(), 1);
    assert_eq!(report.cleanup.records[0].name, "pool");
    assert!(shared.pool().is_closed());
    assert!(matches!(
        database.pool().acquire().await,
        Err(sqlx::Error::PoolClosed)
    ));
}

#[tokio::test]
async fn rejected_fallback_schema_keeps_registered_cleanup_without_closing_shared_pool_early() {
    let mut stack = CleanupStack::new();
    let shared = pool_in(
        stack.reserve("pool").unwrap(),
        PgPoolOptions::new(),
        connection(),
        profile(&["primary", "fallback"]),
    )
    .unwrap();
    assert!(matches!(
        RunledgerDatabase::from_profiled(shared.clone()),
        Err(sqlx::Error::Protocol(_))
    ));
    assert!(!shared.pool().is_closed());
    assert_eq!(stack.len(), 1);
    assert!(stack.close(cleanup()).await.is_success());
    assert!(shared.pool().is_closed());
}

#[tokio::test]
async fn zero_capacity_and_duplicate_reservation_cannot_publish_another_pool() {
    let mut stack = CleanupStack::new();
    assert!(
        pool_in(
            stack.reserve("pool").unwrap(),
            PgPoolOptions::new().max_connections(0),
            connection(),
            profile(&["public"])
        )
        .is_err()
    );
    assert!(stack.is_empty());
    let shared = pool_in(
        stack.reserve("pool").unwrap(),
        PgPoolOptions::new(),
        connection(),
        profile(&["public"]),
    )
    .unwrap();
    assert!(stack.reserve("pool").is_err());
    assert_eq!(stack.len(), 1);
    assert!(stack.close(cleanup()).await.is_success());
    assert!(shared.pool().is_closed());
}

#[tokio::test]
async fn initialization_error_closes_pool_without_a_handoff() {
    let (publish, published) = oneshot::channel();
    let starting = Startup::protected(
        process(),
        OperationOwner::new(BOUND).unwrap().into_context(),
        cleanup(),
        async move |scope| {
            let shared = pool_in(
                scope.reserve_cleanup("pool")?,
                PgPoolOptions::new(),
                connection(),
                profile(&["public"]),
            )?;
            publish.send(shared).unwrap();
            Err::<(), ApplicationError>(std::io::Error::other("initialization refused").into())
        },
    )
    .start();
    let shared = published.await.unwrap();
    let Err(StartupError::Failed(failure)) = starting.finish().await else {
        panic!("expected failure")
    };
    assert!(matches!(failure.cause, StartupCause::Failed(_)));
    assert!(failure.cleanup.is_success());
    assert_eq!(failure.cleanup.records.len(), 1);
    assert!(shared.pool().is_closed());
}

#[tokio::test]
async fn owner_loss_during_initialization_still_drives_registered_close() {
    let (publish, published) = oneshot::channel();
    let starting = Startup::protected(
        process(),
        OperationOwner::new(BOUND).unwrap().into_context(),
        cleanup(),
        async move |scope| {
            let shared = pool_in(
                scope.reserve_cleanup("pool")?,
                PgPoolOptions::new(),
                connection(),
                profile(&["public"]),
            )?;
            publish.send(shared).unwrap();
            std::future::pending::<()>().await;
            Ok::<_, ApplicationError>(())
        },
    )
    .start();
    let shared = published.await.unwrap();
    let observer = starting.observer();
    drop(starting);
    let StartupOutcome::Failed(StartupError::Failed(failure)) =
        tokio::time::timeout(BOUND, observer.wait()).await.unwrap()
    else {
        panic!("expected cancelled startup")
    };
    assert!(matches!(failure.cause, StartupCause::Draining));
    assert!(failure.cleanup.is_success());
    assert_eq!(failure.cleanup.records.len(), 1);
    assert!(shared.pool().is_closed());
}

#[tokio::test]
async fn unstarted_specification_does_not_construct_a_pool() {
    let (publish, published) = oneshot::channel();
    let startup = Startup::protected(
        process(),
        OperationOwner::new(BOUND).unwrap().into_context(),
        cleanup(),
        async move |scope| {
            let shared = pool_in(
                scope.reserve_cleanup("pool")?,
                PgPoolOptions::new(),
                connection(),
                profile(&["public"]),
            )?;
            publish.send(shared).unwrap();
            Ok::<_, ApplicationError>(())
        },
    );
    drop(startup);
    assert!(published.await.is_err());
}

#[cfg(feature = "test-support")]
#[path = "shared_pool/live.rs"]
mod live;
