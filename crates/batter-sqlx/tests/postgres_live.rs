//! Ignored in ordinary gates; scripts/test_sqlx_live.sh requires every case.
mod support;

use batter::{
    BoxError,
    operation::{Interruption, OperationContext, OperationError},
};
use batter_sqlx::{FailureClass, PgLease, SqlxFailure, probe};
use sqlx::Connection;
use std::time::Duration;
use support::{Fixture, Result, bounded, require};
use tokio::sync::oneshot;

#[derive(Clone, Copy)]
enum Stop {
    Cancel,
    Deadline,
    Error,
    Panic,
    Drop,
}

async fn blocked_operation(
    pool: sqlx::PgPool,
    context: OperationContext,
    key: i64,
    pid: oneshot::Sender<i32>,
    stop: oneshot::Receiver<Stop>,
) -> std::result::Result<(), OperationError<BoxError>> {
    context.run("test.blocked", |_| async {
        let mut lease = PgLease::acquire(&pool, &context).await?;
        let backend: i32 = sqlx::query_scalar("SELECT pg_backend_pid()").fetch_one(lease.connection()).await?;
        let _ = pid.send(backend);
        tokio::select! {
            result = sqlx::query("SELECT pg_advisory_lock($1)").bind(key).execute(lease.connection()) => {
                result?;
                Err(std::io::Error::other("blocker unexpectedly released").into())
            }
            signal = stop => match signal {
                Ok(Stop::Panic) => panic!("deliberate application unwind"),
                _ => Err(std::io::Error::other("concrete application failure").into()),
            }
        }
    }).await
}

async fn interrupt(fixture: &mut Fixture, mode: Stop) -> Result {
    let budget = if matches!(mode, Stop::Deadline) {
        Duration::from_secs(5)
    } else {
        Duration::from_secs(30)
    };
    let context = OperationContext::new(budget)?;
    let (pid_tx, pid_rx) = oneshot::channel();
    let (stop_tx, stop_rx) = oneshot::channel();
    let mut task = tokio::spawn(blocked_operation(
        fixture.pool.clone(),
        context.clone(),
        fixture.key,
        pid_tx,
        stop_rx,
    ));
    let body = async {
        let pid = bounded(pid_rx).await??;
        fixture.retired.push(pid);
        fixture.blocked(pid).await?;
        match mode {
            Stop::Cancel => context.cancel(),
            Stop::Drop => task.abort(),
            Stop::Error | Stop::Panic => {
                let _ = stop_tx.send(mode);
            }
            Stop::Deadline => {}
        }
        let result = tokio::time::timeout(Duration::from_secs(7), &mut task).await?;
        match mode {
            Stop::Cancel => require(
                matches!(
                    result,
                    Ok(Err(OperationError::Interrupted(Interruption::Cancelled)))
                ),
                "cancellation classification lost",
            )?,
            Stop::Deadline => require(
                matches!(
                    result,
                    Ok(Err(OperationError::Interrupted(
                        Interruption::DeadlineExceeded
                    )))
                ),
                "deadline classification lost",
            )?,
            Stop::Panic => require(
                matches!(result, Err(error) if error.is_panic()),
                "panic did not propagate",
            )?,
            Stop::Drop => require(
                matches!(result, Err(error) if error.is_cancelled()),
                "outer drop did not cancel task",
            )?,
            Stop::Error => {
                let Ok(Err(OperationError::Failed(error))) = result else {
                    return Err(std::io::Error::other("application error lost").into());
                };
                require(
                    error
                        .downcast_ref::<std::io::Error>()
                        .is_some_and(|e| e.to_string() == "concrete application failure"),
                    "original application cause lost",
                )?;
            }
        }
        // The server still owns this blocked SQL after client task destruction.
        fixture.blocked(pid).await?;
        require(
            fixture.pool.size() == 0,
            "retirement retained pool capacity",
        )?;
        Ok(())
    }
    .await;
    if !task.is_finished() {
        task.abort();
        let cleanup = bounded(&mut task).await.map(|_| ());
        return support::combine(body, cleanup);
    }
    body
}

async fn run_interruption(mode: Stop, repetitions: usize) -> Result {
    let mut fixture = Fixture::new().await?;
    let body = async {
        for _ in 0..repetitions {
            interrupt(&mut fixture, mode).await?;
        }
        require(
            fixture
                .retired
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len()
                == repetitions,
            "backend identities were not independent",
        )?;
        fixture.replacement_and_close().await
    }
    .await;
    fixture.finish(body).await
}

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn blocked_cancellation_releases_capacity() -> Result {
    run_interruption(Stop::Cancel, 1).await
}

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn blocked_deadline_releases_capacity() -> Result {
    run_interruption(Stop::Deadline, 1).await
}

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn blocked_error_releases_capacity() -> Result {
    run_interruption(Stop::Error, 1).await
}

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn blocked_panic_releases_capacity() -> Result {
    run_interruption(Stop::Panic, 1).await
}

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn blocked_outer_drop_releases_capacity() -> Result {
    run_interruption(Stop::Drop, 1).await
}

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn repeated_interruptions_leave_independent_residual_sessions() -> Result {
    run_interruption(Stop::Cancel, 3).await
}

async fn success(fixture: &mut Fixture) -> Result {
    let context = OperationContext::new(Duration::from_secs(10))?;
    let mut lease = PgLease::acquire(&fixture.pool, &context).await?;
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(lease.connection())
        .await?;
    lease.return_to_pool();
    probe(&fixture.pool, &context).await?;
    for commit in [true, false] {
        let mut lease = PgLease::acquire(&fixture.pool, &context).await?;
        let mut tx = lease.connection().begin().await?;
        let actual: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *tx)
            .await?;
        require(actual == pid, "acknowledged success did not reuse backend")?;
        if commit {
            tx.commit().await?;
        } else {
            tx.rollback().await?;
        }
        lease.return_to_pool();
    }
    let actual: i32 =
        bounded(sqlx::query_scalar("SELECT pg_backend_pid()").fetch_one(&fixture.pool)).await??;
    require(
        actual == pid,
        "acknowledged transaction did not permit reuse",
    )?;
    bounded(fixture.pool.close()).await?;
    Ok(())
}

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn success_and_acknowledged_transactions_reuse() -> Result {
    let mut fixture = Fixture::new().await?;
    let body = success(&mut fixture).await;
    fixture.finish(body).await
}

async fn native_failure(fixture: &mut Fixture) -> Result {
    let context = OperationContext::new(Duration::from_secs(10))?;
    let mut lease = PgLease::acquire(&fixture.pool, &context).await?;
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(lease.connection())
        .await?;
    fixture.retired.push(pid);
    let error = sqlx::query("SELECT 1 / 0 /* secret-query-marker */")
        .execute(lease.connection())
        .await
        .err()
        .ok_or_else(|| std::io::Error::other("expected database failure"))?;
    let error = SqlxFailure::from(error);
    require(
        error.class() == FailureClass::Database,
        "database classification lost",
    )?;
    require(
        error
            .native()
            .as_database_error()
            .and_then(|e| e.code())
            .as_deref()
            == Some("22012"),
        "SQLSTATE lost",
    )?;
    require(
        format!("{error:?} {error}") == "PostgreSQL operation failed PostgreSQL operation failed",
        "diagnostics were not fixed",
    )?;
    drop(lease);
    let next: i32 =
        bounded(sqlx::query_scalar("SELECT pg_backend_pid()").fetch_one(&fixture.pool)).await??;
    require(next != pid, "failed connection was reused")?;
    Ok(())
}

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn native_database_failure_retires_and_preserves_cause() -> Result {
    let mut fixture = Fixture::new().await?;
    let body = native_failure(&mut fixture).await;
    fixture.finish(body).await
}

async fn commit_failure(fixture: &mut Fixture) -> Result {
    let context = OperationContext::new(Duration::from_secs(10))?;
    let mut lease = PgLease::acquire(&fixture.pool, &context).await?;
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(lease.connection())
        .await?;
    fixture.retired.push(pid);
    let mut tx = lease.connection().begin().await?;
    sqlx::query("CREATE TEMP TABLE disposition_commit (value integer UNIQUE DEFERRABLE INITIALLY DEFERRED) ON COMMIT DROP")
        .execute(&mut *tx).await?;
    sqlx::query("INSERT INTO disposition_commit VALUES (1), (1)")
        .execute(&mut *tx)
        .await?;
    let error =
        tx.commit().await.err().ok_or_else(|| {
            std::io::Error::other("expected deferred constraint failure at commit")
        })?;
    let error = SqlxFailure::from(error);
    require(
        error
            .native()
            .as_database_error()
            .and_then(|e| e.code())
            .as_deref()
            == Some("23505"),
        "commit SQLSTATE lost",
    )?;
    require(
        error.class() == FailureClass::Database,
        "commit error was reclassified as interruption",
    )?;
    drop(lease);
    let next: i32 =
        bounded(sqlx::query_scalar("SELECT pg_backend_pid()").fetch_one(&fixture.pool)).await??;
    require(next != pid, "unsuccessful commit returned its connection")?;
    Ok(())
}

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn rejected_commit_preserves_native_cause_and_retires() -> Result {
    let mut fixture = Fixture::new().await?;
    let body = bounded(commit_failure(&mut fixture))
        .await
        .and_then(|result| result);
    fixture.finish(body).await
}

async fn ordinary_return(fixture: &mut Fixture) -> Result {
    let mut connection = bounded(fixture.pool.acquire()).await??;
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *connection)
        .await?;
    fixture.retired.push(pid);
    let (stop_tx, stop_rx) = oneshot::channel::<()>();
    let key = fixture.key;
    let mut task = tokio::spawn(async move {
        tokio::select! {
            _ = stop_rx => {},
            _ = sqlx::query("SELECT pg_advisory_lock($1)").bind(key).execute(&mut *connection) => {},
        }
        // The original SQLx pool-return behavior: asynchronous health ping.
        drop(connection);
    });
    let body = async {
        fixture.blocked(pid).await?;
        let _ = stop_tx.send(());
        bounded(&mut task).await??;
        require(
            fixture.pool.size() == 1,
            "ordinary return unexpectedly retired accounting",
        )?;
        require(
            matches!(fixture.pool.acquire().await, Err(sqlx::Error::PoolTimedOut)),
            "ordinary return did not retain blocked slot",
        )?;
        require(
            tokio::time::timeout(Duration::from_millis(200), fixture.pool.close())
                .await
                .is_err(),
            "ordinary pool close unexpectedly completed before unlock",
        )?;
        fixture.blocked(pid).await
    }
    .await;
    if !task.is_finished() {
        task.abort();
        return support::combine(body, bounded(&mut task).await.map(|_| ()));
    }
    body
}

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn ordinary_return_control_retains_blocked_capacity() -> Result {
    let mut fixture = Fixture::new().await?;
    let body = ordinary_return(&mut fixture).await;
    fixture.finish(body).await
}
