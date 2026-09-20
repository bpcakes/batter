//! Native migration behavior under opaque lease ownership.
mod support;
use batter_core::operation::OperationContext;
use batter_sqlx::PgLease;
use sqlx::{
    Connection, SqlSafeStr,
    migrate::{Migration, MigrationType, Migrator},
};
use std::time::Duration;
use support::{Fixture, Result, bounded, combine, require};

fn migrator(ledger: &str, statement: sqlx::SqlStr) -> Migrator {
    let mut migrator = Migrator::with_migrations(vec![Migration::new(
        1,
        "owned migration".into(),
        MigrationType::Simple,
        statement,
        false,
    )]);
    migrator.dangerous_set_table_name(ledger.to_owned());
    migrator
}

async fn cleanup_ledger(ledger: &str) -> Result {
    let mut connection = sqlx::PgConnection::connect(&std::env::var("DATABASE_URL")?).await?;
    let result = sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "DROP TABLE IF EXISTS {ledger}"
    )))
    .execute(&mut connection)
    .await
    .map(|_| ())
    .map_err(Into::into);
    combine(result, connection.close().await.map_err(Into::into))
}

#[tokio::test]
#[ignore = "requires external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn migration_preserves_native_history_and_retires_checksum_failure() -> Result {
    let mut fixture = Fixture::new().await?;
    let ledger = format!("batter_migration_{}", fixture.key);
    let context = OperationContext::new(Duration::from_secs(30))?;
    let body = async {
        let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&fixture.pool)
            .await?;
        let bundle = migrator(&ledger, "SELECT 1".into_sql_str());
        PgLease::acquire(&fixture.pool, &context)
            .await?
            .migrate(&bundle)
            .await?;
        let reused: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&fixture.pool)
            .await?;
        require(pid == reused, "successful migration lost pool reuse")?;
        // Native history makes a second exact application a no-op.
        PgLease::acquire(&fixture.pool, &context)
            .await?
            .migrate(&bundle)
            .await?;
        let changed = migrator(&ledger, "SELECT 2".into_sql_str());
        let error = PgLease::acquire(&fixture.pool, &context)
            .await?
            .migrate(&changed)
            .await
            .expect_err("native checksum mismatch must fail");
        require(
            matches!(error.native(), sqlx::Error::Migrate(cause)
            if matches!(cause.as_ref(), sqlx::migrate::MigrateError::VersionMismatch(1))),
            "native checksum cause was not preserved",
        )?;
        fixture.retired.push(pid);
        let replacement: i32 =
            bounded(sqlx::query_scalar("SELECT pg_backend_pid()").fetch_one(&fixture.pool))
                .await??;
        require(
            pid != replacement,
            "failed migration returned a possibly locked session",
        )?;
        Ok(())
    }
    .await;
    let result = fixture.finish(body).await;
    combine(result, cleanup_ledger(&ledger).await)
}

#[tokio::test]
#[ignore = "requires external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn interrupted_migration_retires_without_waiting_for_server_lock() -> Result {
    let mut fixture = Fixture::new().await?;
    let ledger = format!("batter_migration_cancel_{}", fixture.key);
    let context = OperationContext::new(Duration::from_secs(30))?;
    let body = async {
        let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&fixture.pool)
            .await?;
        let bundle = migrator(
            &ledger,
            sqlx::AssertSqlSafe(format!("SELECT pg_advisory_lock({})", fixture.key)).into_sql_str(),
        );
        let lease = PgLease::acquire(&fixture.pool, &context).await?;
        let mut operation = Box::pin(lease.migrate(&bundle));
        tokio::select! {
            result = &mut operation => { result?; return Err("migration did not block".into()); }
            result = fixture.blocked(pid) => result?,
        }
        drop(operation);
        fixture.retired.push(pid);
        fixture.replacement_and_close().await
    }
    .await;
    let result = fixture.finish(body).await;
    combine(result, cleanup_ledger(&ledger).await)
}
