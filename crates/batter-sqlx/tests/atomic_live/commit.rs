use super::*;
use sqlx::Connection;

#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn commit_in_flight_cancellation_and_disconnect_are_uncertain() -> Result {
    for cancel in [true, false] {
        let mut fixture = fixture().await?;
        let table = format!("batter_commit_{}", fixture.key);
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            "CREATE TABLE {table}(id integer UNIQUE DEFERRABLE INITIALLY DEFERRED)"
        )))
        .execute(&mut fixture.observer)
        .await?;
        let mut blocker = sqlx::PgConnection::connect(&std::env::var("DATABASE_URL")?).await?;
        let mut blocking = blocker.begin().await?;
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            "INSERT INTO {table} VALUES (1)"
        )))
        .execute(&mut *blocking)
        .await?;
        let body = async {
            let (tx, previous) = transaction(&fixture.pool).await?;
            let (tx, ()) = tx
                .application(async |sql| {
                    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
                        "INSERT INTO {table} VALUES (1)"
                    )))
                    .execute(sql.executor())
                    .await?;
                    Ok::<_, sqlx::Error>(())
                })
                .await?;
            let task = tokio::spawn(tx.commit());
            fixture.blocked(previous).await?;
            let query: String =
                sqlx::query_scalar("SELECT query FROM pg_stat_activity WHERE pid=$1")
                    .bind(previous)
                    .fetch_one(&mut fixture.observer)
                    .await?;
            assert_eq!(
                query, "COMMIT",
                "fault must occur during COMMIT, not validation"
            );
            if cancel {
                fixture.retired.push(previous);
                task.abort();
                assert!(support::bounded(task).await?.unwrap_err().is_cancelled());
                fixture.replacement_and_close().await?;
            } else {
                sqlx::query("SELECT pg_terminate_backend($1)")
                    .bind(previous)
                    .execute(&mut fixture.observer)
                    .await?;
                let _unconfirmed = support::bounded(task).await??.unwrap_err();
                replacement(&fixture.pool, previous).await?;
            }
            Ok(())
        }
        .await;
        blocking.rollback().await?;
        blocker.close().await?;
        // Cleanup may wait for the retired COMMIT to settle after releasing its
        // blocker. It does not infer rollback from client cancellation.
        support::bounded(
            sqlx::raw_sql(sqlx::AssertSqlSafe(format!("DROP TABLE {table}")))
                .execute(&mut fixture.observer),
        )
        .await??;
        fixture.finish(body).await?;
    }
    Ok(())
}
