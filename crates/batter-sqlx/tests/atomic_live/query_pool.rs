use super::{fixture, support::Result};
use batter_core::operation::{Interruption, OperationContext, OperationError};
use batter_sqlx::PgQueryHandle;
use sqlx::Row;
use std::{
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};

fn send<T: Send>(value: T) -> T {
    value
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn pooled_query_helpers_preserve_macros_reuse_and_error_mapping() -> Result {
    let fixture = fixture().await?;
    let body = async {
        let context = OperationContext::new(Duration::from_secs(10))?;
        let mapped = AtomicUsize::new(0);
        let queries = PgQueryHandle::within(
            &fixture.pool,
            &context,
            Duration::from_secs(8),
            "queries.native",
            |error| {
                mapped.fetch_add(1, Ordering::SeqCst);
                error
            },
        )?;
        let first: i32 = queries
            .fetch_one(sqlx::query_scalar("SELECT pg_backend_pid()"))
            .await?;
        macro_shapes(&queries).await?;
        macro_commands(&queries).await?;
        let same: i32 = queries
            .fetch_one(sqlx::query_scalar("SELECT pg_backend_pid()"))
            .await?;
        assert_eq!(first, same, "successful cleanup permits native reuse");
        let error = queries
            .fetch_one(sqlx::query!("DO $$ BEGIN NULL; END $$"))
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            OperationError::Failed(sqlx::Error::RowNotFound)
        ));
        assert_eq!(mapped.load(Ordering::SeqCst), 1);
        let replacement: i32 = queries
            .fetch_one(sqlx::query_scalar("SELECT pg_backend_pid()"))
            .await?;
        assert_ne!(replacement, first, "failed query retires its lease");
        assert!(
            context.check().is_ok(),
            "child completion must not cancel parent"
        );
        Ok(())
    }
    .await;
    fixture.finish(body).await
}

async fn macro_shapes<M>(
    queries: &PgQueryHandle<'_, M>,
) -> std::result::Result<(), OperationError<sqlx::Error>>
where
    M: Fn(OperationError<sqlx::Error>) -> OperationError<sqlx::Error> + Sync,
{
    assert_eq!(
        send(queries.fetch_one(sqlx::query_scalar!("SELECT 42::bigint AS \"value!\""))).await?,
        42
    );
    let row = queries
        .fetch_one(sqlx::query!("SELECT 42::bigint AS \"value!\""))
        .await?;
    assert_eq!(row.value, 42);
    #[derive(Debug)]
    struct Record {
        value: i64,
    }
    assert_eq!(
        queries
            .fetch_one(sqlx::query_as!(Record, "SELECT 42::bigint AS \"value!\""))
            .await?
            .value,
        42
    );
    assert!(
        queries
            .fetch_optional(sqlx::query!("SELECT 42::bigint AS \"value!\" WHERE false"))
            .await?
            .is_none()
    );
    assert!(
        queries
            .fetch_optional(sqlx::query_scalar!(
                "SELECT 42::bigint AS \"value!\" WHERE false"
            ))
            .await?
            .is_none()
    );
    assert_eq!(
        queries
            .fetch_all(sqlx::query!("SELECT 42::bigint AS \"value!\""))
            .await?[0]
            .value,
        42
    );
    assert_eq!(
        queries
            .fetch_all(sqlx::query_scalar!("SELECT 42::bigint AS \"value!\""))
            .await?,
        vec![42]
    );
    Ok(())
}

async fn macro_commands<M>(
    queries: &PgQueryHandle<'_, M>,
) -> std::result::Result<(), OperationError<sqlx::Error>>
where
    M: Fn(OperationError<sqlx::Error>) -> OperationError<sqlx::Error> + Sync,
{
    assert!(
        queries
            .fetch_optional(sqlx::query!("DO $$ BEGIN NULL; END $$"))
            .await?
            .is_none()
    );
    assert!(
        queries
            .fetch_all(sqlx::query!("DO $$ BEGIN NULL; END $$"))
            .await?
            .is_empty()
    );
    assert_eq!(
        queries
            .execute(sqlx::query!("DO $$ BEGIN NULL; END $$"))
            .await?
            .rows_affected(),
        0
    );
    assert_eq!(
        queries
            .execute(sqlx::query!("SELECT 42::bigint AS \"value!\""))
            .await?
            .rows_affected(),
        1
    );
    assert_eq!(
        queries
            .execute(sqlx::query_scalar!("SELECT 42::bigint AS \"value!\""))
            .await?
            .rows_affected(),
        1
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn pooled_query_acquisition_and_blocked_query_cancellation_preserve_ownership() -> Result {
    let mut fixture = fixture().await?;
    let pool = fixture.pool.clone();
    let body = async {
        let context = OperationContext::new(Duration::from_secs(10))?;
        let queries = PgQueryHandle::within(
            &pool,
            &context,
            Duration::from_secs(8),
            "queries.acquire",
            std::convert::identity,
        )?;
        let mut held = pool.acquire().await?;
        let mut waiting =
            Box::pin(queries.fetch_one(sqlx::query_scalar::<_, i64>("SELECT 42::bigint")));
        assert!(futures_util::poll!(&mut waiting).is_pending());
        context.cancel();
        assert!(matches!(
            waiting.await,
            Err(OperationError::Interrupted(Interruption::Cancelled))
        ));
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT 42::bigint")
                .fetch_one(&mut *held)
                .await?,
            42
        );
        drop(held);
        let context = OperationContext::new(Duration::from_secs(10))?;
        let queries = PgQueryHandle::within(
            &pool,
            &context,
            Duration::from_secs(8),
            "queries.blocked",
            std::convert::identity,
        )?;
        let previous = queries
            .fetch_one(sqlx::query_scalar::<_, i32>("SELECT pg_backend_pid()"))
            .await?;
        fixture.retired.push(previous);
        let mut blocked =
            Box::pin(queries.execute(sqlx::query("SELECT pg_advisory_lock($1)").bind(fixture.key)));
        tokio::select! {
            result = &mut blocked => panic!("query did not block: {result:?}"),
            result = fixture.blocked(previous) => result?,
        }
        context.cancel();
        assert!(matches!(
            blocked.await,
            Err(OperationError::Interrupted(Interruption::Cancelled))
        ));
        fixture.replacement_and_close().await?;
        Ok(())
    }
    .await;
    fixture.finish(body).await
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn pooled_query_observed_output_survives_cleanup_cancellation() -> Result {
    let fixture = fixture().await?;
    let body = async {
        let context = OperationContext::new(Duration::from_secs(10))?;
        let mapped = AtomicUsize::new(0);
        let queries = PgQueryHandle::within(
            &fixture.pool,
            &context,
            Duration::from_secs(8),
            "queries.retain",
            |error| {
                mapped.fetch_add(1, Ordering::SeqCst);
                error
            },
        )?;
        let value = queries
            .fetch_one(sqlx::query("SELECT 42::bigint AS value").map(
                |row: sqlx::postgres::PgRow| {
                    // SQLx has observed the native result. Its mapper cancels before
                    // the lease's asynchronous BEGIN/ROLLBACK return handshake.
                    context.cancel();
                    row.get::<i64, _>("value")
                },
            ))
            .await?;
        assert_eq!(
            value, 42,
            "cleanup interruption cannot erase observed output"
        );
        assert_eq!(mapped.load(Ordering::SeqCst), 0);
        assert!(matches!(context.check(), Err(Interruption::Cancelled)));
        let later = queries
            .execute(sqlx::query!("DO $$ BEGIN NULL; END $$"))
            .await;
        assert!(matches!(
            later,
            Err(OperationError::Interrupted(Interruption::Cancelled))
        ));
        assert_eq!(mapped.load(Ordering::SeqCst), 1);
        Ok(())
    }
    .await;
    fixture.finish(body).await
}
