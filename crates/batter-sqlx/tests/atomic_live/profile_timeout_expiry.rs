use super::{
    fixture,
    profile_timeouts::complete,
    support::{OBSERVE, Result, bounded, require},
};
use batter_sqlx::PgProfiledPool;
use sqlx::postgres::PgPoolOptions;
use std::time::Duration;

async fn expiration(idle: bool) -> Result {
    let mut fixture = fixture().await?;
    let options = fixture.pool.connect_options().as_ref().clone();
    let limit = Duration::from_millis(750);
    let profile = complete(
        options.get_username(),
        if idle { limit } else { Duration::ZERO },
        if idle { Duration::ZERO } else { limit },
    );
    bounded(fixture.pool.close()).await?;
    let database =
        PgProfiledPool::connect_lazy(options, profile, PgPoolOptions::new().max_connections(1))?;
    fixture.pool = database.pool().clone();
    let body = async {
        let mut connection = bounded(database.pool().acquire()).await??;
        let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *connection)
            .await?;
        sqlx::raw_sql("BEGIN").execute(&mut *connection).await?;
        let key = fixture.key + 1;
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(key)
            .execute(&mut *connection)
            .await?;
        let held: bool = sqlx::query_scalar("SELECT pg_try_advisory_lock($1)")
            .bind(key)
            .fetch_one(&mut fixture.observer)
            .await?;
        require(
            !held,
            "transaction must hold the independently observed lock",
        )?;
        let error = if idle {
            // Keep the Rust lease alive while the observer watches server expiry.
            disappeared(&mut fixture.observer, pid).await?;
            sqlx::query("SELECT 1")
                .execute(&mut *connection)
                .await
                .unwrap_err()
        } else {
            tokio::time::timeout(OBSERVE, async {
                let mut completed = 0;
                loop {
                    match sqlx::query("SELECT pg_sleep(0.01)")
                        .execute(&mut *connection)
                        .await
                    {
                        Ok(_) => completed += 1,
                        Err(error) => {
                            require(
                                completed >= 2,
                                "must exceed a total budget across short statements",
                            )?;
                            break Ok::<_, batter_core::BoxError>(error);
                        }
                    }
                }
            })
            .await??
        };
        assert_eq!(
            error.as_database_error().and_then(|e| e.code()).as_deref(),
            Some(if idle { "25P03" } else { "25P04" })
        );
        disappeared(&mut fixture.observer, pid).await?;
        let released: bool = sqlx::query_scalar("SELECT pg_try_advisory_lock($1)")
            .bind(key)
            .fetch_one(&mut fixture.observer)
            .await?;
        require(
            released,
            "server termination must release the transaction lock",
        )?;
        sqlx::query("SELECT pg_advisory_unlock($1)")
            .bind(key)
            .execute(&mut fixture.observer)
            .await?;
        assert_eq!(database.pool().size(), 1);
        assert_eq!(database.pool().num_idle(), 0);
        assert!(
            database.pool().try_acquire().is_none(),
            "server expiry cannot release a held Rust lease"
        );
        drop(connection);
        let next: i32 =
            bounded(sqlx::query_scalar("SELECT pg_backend_pid()").fetch_one(database.pool()))
                .await??;
        assert_ne!(next, pid);
        Ok(())
    }
    .await;
    fixture.finish(body).await
}

async fn disappeared(observer: &mut sqlx::PgConnection, pid: i32) -> Result {
    tokio::time::timeout(OBSERVE, async {
        loop {
            let exists: bool =
                sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM pg_stat_activity WHERE pid = $1)")
                    .bind(pid)
                    .fetch_one(&mut *observer)
                    .await?;
            if !exists {
                break Ok::<_, sqlx::Error>(());
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await??;
    Ok(())
}

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn declared_idle_timeout_releases_server_locks_but_not_held_lease() -> Result {
    expiration(true).await
}

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn declared_transaction_timeout_bounds_multiple_short_statements() -> Result {
    expiration(false).await
}
