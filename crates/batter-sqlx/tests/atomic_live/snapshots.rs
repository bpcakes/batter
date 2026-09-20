use super::*;
use batter_sqlx::{PgReadOnlySnapshot, PgSnapshotError};

#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn snapshot_is_coherent_read_only_and_normalized() -> Result {
    let mut fixture = fixture().await?;
    let table = format!("batter_snapshot_{}", fixture.key);
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "CREATE TABLE {table}(id integer); INSERT INTO {table} VALUES (1)"
    )))
    .execute(&mut fixture.observer)
    .await?;
    let body = async {
        // Leave an untracked transaction with uncommitted data in the pool.
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!("BEGIN; INSERT INTO {table} VALUES (2)")))
            .execute(&fixture.pool).await?;
        let counts = PgReadOnlySnapshot::inspect(&fixture.pool, async |sql| {
            let settings: (String, String) = sqlx::query_as("SELECT current_setting('transaction_isolation'), current_setting('transaction_read_only')")
                .fetch_one(sql.executor()).await?;
            assert_eq!(settings, ("repeatable read".into(), "on".into()));
            let query = format!("SELECT count(*) FROM public.{table}");
            let first: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(query.clone())).fetch_one(sql.executor()).await?;
            sqlx::raw_sql(sqlx::AssertSqlSafe(format!("INSERT INTO {table} VALUES (3)"))).execute(&mut fixture.observer).await?;
            let second: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(query)).fetch_one(sql.executor()).await?;
            Ok::<_, sqlx::Error>((first, second))
        }).await?;
        assert_eq!(counts, (1, 1));
        let count: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT count(*) FROM {table}")))
            .fetch_one(&mut fixture.observer).await?;
        assert_eq!(count, 2);
        let error = PgReadOnlySnapshot::inspect(&fixture.pool, async |sql| {
            sqlx::raw_sql(sqlx::AssertSqlSafe(format!("INSERT INTO public.{table} VALUES (4)"))).execute(sql.executor()).await
        }).await.unwrap_err();
        assert!(matches!(error, PgSnapshotError::Inspection(ref native) if native.as_database_error().unwrap().code().as_deref() == Some("25006")));
        Ok(())
    }.await;
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("DROP TABLE {table}")))
        .execute(&mut fixture.observer)
        .await?;
    fixture.finish(body).await
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn snapshot_transaction_control_never_returns_evidence() -> Result {
    let fixture = fixture().await?;
    let body = async {
        for statement in [
            "COMMIT",
            "ROLLBACK",
            "COMMIT AND CHAIN",
            "ROLLBACK AND CHAIN",
            "COMMIT; BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY",
        ] {
            let (sender, receiver) = tokio::sync::oneshot::channel();
            let result = PgReadOnlySnapshot::inspect(&fixture.pool, async |sql| {
                let _ = sender.send(pid(sql).await?);
                sqlx::raw_sql(statement).execute(sql.executor()).await?;
                Ok::<_, sqlx::Error>(())
            })
            .await;
            assert!(result.is_err(), "snapshot boundary survived {statement}");
            replacement(&fixture.pool, receiver.await?).await?;
        }
        Ok(())
    }
    .await;
    fixture.finish(body).await
}
