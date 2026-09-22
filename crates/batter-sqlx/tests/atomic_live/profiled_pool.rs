use super::{
    fixture,
    support::{Result, bounded},
};
use batter_core::operation::OperationContext;
use batter_sqlx::{PgProfiledPool, PgSessionProfile, run_atomic_profiled_in};
use sqlx::postgres::PgPoolOptions;
use std::time::Duration;

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn owned_pool_replaces_hooks_and_restores_fast_acquisition_and_atomic_policy() -> Result {
    let fixture = fixture().await?;
    let body = async {
        let options = fixture.pool.connect_options();
        let login = options.get_username();
        let profile = PgSessionProfile::new(login, login, vec!["public".into()],
            Duration::from_secs(2), Duration::from_millis(250))?
            .with_setting("app.tenant", "expected")?;
        let database = PgProfiledPool::connect_lazy((*options).clone(), profile,
            PgPoolOptions::new().max_connections(1)
                .after_connect(|_, _| Box::pin(async { panic!("replaced connect hook") }))
                .before_acquire(|_, _| Box::pin(async { panic!("replaced acquire hook") }))
                .after_release(|_, _| Box::pin(async { panic!("replaced release hook") })))?;
        let mut connection = bounded(database.pool().acquire()).await??;
        sqlx::raw_sql("SET app.tenant = 'contaminated'; SET search_path = pg_catalog")
            .execute(&mut *connection).await?;
        drop(connection);
        let mut connection = bounded(async {
            loop {
                if let Some(connection) = database.pool().try_acquire() {
                    break connection;
                }
                tokio::task::yield_now().await;
            }
        }).await?;
        let observed: (String, String) = sqlx::query_as(
            "SELECT current_schema()::text, current_setting('app.tenant')")
            .fetch_one(&mut *connection).await?;
        assert_eq!(observed, ("public".into(), "expected".into()));
        drop(connection);
        let context = batter_core::operation::OperationOwner::new(Duration::from_secs(5))?.into_context();
        let observed = run_atomic_profiled_in(&database, &context, "test.profiled", async |scope| {
            scope.application(async |sql| {
                sqlx::query_as::<_, (String, String, String)>(
                    "SELECT current_user::text, current_schema()::text, current_setting('app.tenant')")
                    .fetch_one(sql.executor()).await
            }).await
        }).await?;
        assert_eq!(observed, (login.into(), "public".into(), "expected".into()));
        database.pool().close().await;
        Ok(())
    }.await;
    fixture.finish(body).await
}
