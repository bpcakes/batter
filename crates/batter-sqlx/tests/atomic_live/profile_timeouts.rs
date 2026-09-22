use super::{
    fixture,
    support::{Result, bounded},
};
use batter_sqlx::{PgProfiledPool, PgReadOnlySnapshot, PgSessionProfile, run_atomic_profiled};
use sqlx::{PgConnection, postgres::PgPoolOptions};
use std::time::Duration;

pub(super) const VALUES: &str = "SELECT \
    (SELECT setting::bigint FROM pg_settings WHERE name = 'idle_in_transaction_session_timeout'), \
    (SELECT setting::bigint FROM pg_settings WHERE name = 'transaction_timeout')";

pub(super) fn complete(login: &str, idle: Duration, total: Duration) -> PgSessionProfile {
    PgSessionProfile::with_timeouts(
        login,
        login,
        vec!["public".into()],
        Duration::from_secs(2),
        Duration::from_millis(250),
        idle,
        total,
    )
    .unwrap()
}

async fn values(connection: &mut PgConnection) -> Result<(i64, i64)> {
    Ok(sqlx::query_as(VALUES).fetch_one(connection).await?)
}

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn transaction_profiles_distinguish_startup_defaults_session_settings_and_zero() -> Result {
    let mut fixture = fixture().await?;
    let options = fixture.pool.connect_options().as_ref().clone().options([
        ("idle_in_transaction_session_timeout", "10s"),
        ("transaction_timeout", "15s"),
    ]);
    bounded(fixture.pool.close()).await?;
    fixture.pool = PgPoolOptions::new().max_connections(1)
        .after_connect(|connection, _| Box::pin(async move {
            sqlx::raw_sql("SET idle_in_transaction_session_timeout = '20s'; SET transaction_timeout = '30s'")
                .execute(connection).await?;
            Ok(())
        })).connect_lazy_with(options);
    let body = async {
        let login = fixture.pool.connect_options().get_username().to_owned();
        let mut connection = bounded(fixture.pool.acquire()).await??;
        assert_eq!(values(&mut connection).await?, (20_000, 30_000));
        let legacy = PgSessionProfile::new(
            &login,
            &login,
            vec!["public".into()],
            Duration::ZERO,
            Duration::ZERO,
        )?;
        legacy.reset_and_apply(&mut connection).await?;
        assert_eq!(values(&mut connection).await?, (10_000, 15_000));
        for (idle, total) in [(11_000, 22_000), (0, 22_000), (11_000, 0), (0, 0)] {
            complete(
                &login,
                Duration::from_millis(idle),
                Duration::from_millis(total),
            )
            .reset_and_apply(&mut connection)
            .await?;
            assert_eq!(values(&mut connection).await?, (idle as i64, total as i64));
        }
        // The compatibility path must not turn an undeclared value into explicit zero.
        legacy.reset_and_apply(&mut connection).await?;
        assert_eq!(values(&mut connection).await?, (10_000, 15_000));
        Ok(())
    }
    .await;
    fixture.finish(body).await
}

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn transaction_profiles_cover_pool_hooks_atomic_and_snapshot_work() -> Result {
    let mut fixture = fixture().await?;
    let options = fixture.pool.connect_options().as_ref().clone();
    let profile = complete(
        options.get_username(),
        Duration::from_secs(11),
        Duration::from_secs(22),
    );
    bounded(fixture.pool.close()).await?;
    let database = PgProfiledPool::connect_lazy(
        options,
        profile,
        PgPoolOptions::new()
            .max_connections(1)
            .after_connect(|_, _| Box::pin(async { panic!("replaced connect hook") }))
            .before_acquire(|_, _| Box::pin(async { panic!("replaced acquire hook") }))
            .after_release(|_, _| Box::pin(async { panic!("replaced release hook") })),
    )?;
    fixture.pool = database.pool().clone();
    let body = async {
        let mut original_pid = None;
        for fast in [false, true, false] {
            let mut connection = if fast {
                bounded(async {
                    loop {
                        if let Some(connection) = database.pool().try_acquire() {
                            break connection;
                        }
                        tokio::task::yield_now().await;
                    }
                })
                .await?
            } else {
                bounded(database.pool().acquire()).await??
            };
            let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
                .fetch_one(&mut *connection)
                .await?;
            assert_eq!(
                pid,
                *original_pid.get_or_insert(pid),
                "must restore the same physical session"
            );
            assert_eq!(values(&mut connection).await?, (11_000, 22_000));
            sqlx::raw_sql(
                "SET idle_in_transaction_session_timeout = 0; SET transaction_timeout = 0",
            )
            .execute(&mut *connection)
            .await?;
            drop(connection);
        }
        let atomic: (i64, i64) =
            run_atomic_profiled(database.pool(), database.profile(), async |scope| {
                scope
                    .application(async |sql| sqlx::query_as(VALUES).fetch_one(sql.executor()).await)
                    .await
            })
            .await?;
        assert_eq!(atomic, (11_000, 22_000));
        let snapshot: (i64, i64) = PgReadOnlySnapshot::inspect_profiled(
            database.pool(),
            database.profile(),
            async |sql| sqlx::query_as(VALUES).fetch_one(sql.executor()).await,
        )
        .await?;
        assert_eq!(snapshot, (11_000, 22_000));
        Ok(())
    }
    .await;
    fixture.finish(body).await
}
