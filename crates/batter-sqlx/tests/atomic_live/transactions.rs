use super::*;
use batter_sqlx::{PgScopeError, PgTransactionError};

#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn transaction_control_is_terminal() -> Result {
    let fixture = fixture().await?;
    let body = async {
        for statement in [
            "COMMIT",
            "ROLLBACK",
            "COMMIT AND CHAIN",
            "ROLLBACK AND CHAIN",
        ] {
            let (tx, previous) = transaction(&fixture.pool).await?;
            let result = tx
                .application(async |sql| {
                    sqlx::raw_sql(statement).execute(sql.executor()).await?;
                    Ok::<_, sqlx::Error>(())
                })
                .await;
            assert!(
                matches!(
                    result,
                    Err(PgScopeError::Transaction(
                        PgTransactionError::TransactionBoundaryLost
                    ))
                ),
                "{statement}: {result:?}"
            );
            replacement(&fixture.pool, previous).await?;
        }
        Ok(())
    }
    .await;
    fixture.finish(body).await
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn application_and_library_operations_commit_together() -> Result {
    let mut fixture = fixture().await?;
    let table = format!("batter_atomic_{}", fixture.key);
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "CREATE TABLE {table}(id integer PRIMARY KEY)"
    )))
    .execute(&mut fixture.observer)
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
        let (tx, outcome) = library_insert(tx, &table, 2).await?;
        outcome?;
        let invisible: i64 =
            sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT count(*) FROM {table}")))
                .fetch_one(&mut fixture.observer)
                .await?;
        assert_eq!(invisible, 0);
        let _confirmed = tx.commit().await?;
        let visible: i64 =
            sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT count(*) FROM {table}")))
                .fetch_one(&mut fixture.observer)
                .await?;
        assert_eq!(visible, 2);
        let same: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&fixture.pool)
            .await?;
        assert_eq!(same, previous, "acknowledged commit should permit reuse");
        let (tx, result) =
            library_insert(PgAtomicTransaction::begin(&fixture.pool).await?, &table, 3).await?;
        result?;
        let _rolled_back = tx.rollback().await?;
        let visible: i64 =
            sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT count(*) FROM {table}")))
                .fetch_one(&mut fixture.observer)
                .await?;
        assert_eq!(visible, 2);
        Ok(())
    }
    .await;
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("DROP TABLE {table}")))
        .execute(&mut fixture.observer)
        .await?;
    fixture.finish(body).await
}

// This integration-test crate is a real downstream consumer: no crate-private
// resource access or dependency on Runledger is available to the implementation.
async fn library_insert(
    tx: PgAtomicTransaction,
    table: &str,
    id: i32,
) -> std::result::Result<
    (PgAtomicTransaction, std::result::Result<(), sqlx::Error>),
    PgScopeError<sqlx::Error>,
> {
    tx.operation(async |sql| {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "INSERT INTO {table} VALUES ($1)"
        )))
        .bind(id)
        .execute(sql.executor())
        .await?;
        Ok(())
    })
    .await
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn savepoint_errors_recover_without_partial_writes() -> Result {
    let fixture = fixture().await?;
    let body = async {
        sqlx::raw_sql("CREATE TEMP TABLE atomic_rows(id integer PRIMARY KEY)")
            .execute(&fixture.pool)
            .await?;
        let tx = PgAtomicTransaction::begin(&fixture.pool).await?;
        let (tx, ()) = tx
            .application(async |sql| {
                sqlx::raw_sql("INSERT INTO atomic_rows VALUES (1); SAVEPOINT application_previous")
                    .execute(sql.executor())
                    .await?;
                Ok::<_, sqlx::Error>(())
            })
            .await?;
        let (tx, result) = library_insert(tx, "atomic_rows", 2).await?;
        result?;
        let (tx, result) = tx
            .operation(async |sql| {
                sqlx::raw_sql("ROLLBACK TO SAVEPOINT application_previous")
                    .execute(sql.executor())
                    .await
            })
            .await?;
        assert_eq!(
            result
                .unwrap_err()
                .as_database_error()
                .unwrap()
                .code()
                .as_deref(),
            Some("3B001")
        );
        let (tx, result) = tx
            .operation(async |sql| {
                sqlx::raw_sql(
                    "INSERT INTO atomic_rows VALUES (3); INSERT INTO atomic_rows VALUES (1)",
                )
                .execute(sql.executor())
                .await
            })
            .await?;
        assert_eq!(
            result
                .unwrap_err()
                .as_database_error()
                .unwrap()
                .code()
                .as_deref(),
            Some("23505")
        );
        let _committed = tx.commit().await?;
        let rows: Vec<i32> = sqlx::query_scalar("SELECT id FROM atomic_rows ORDER BY id")
            .fetch_all(&fixture.pool)
            .await?;
        assert_eq!(rows, vec![1, 2]);
        Ok(())
    }
    .await;
    fixture.finish(body).await
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn swallowed_sql_error_and_application_error_are_terminal() -> Result {
    let fixture = fixture().await?;
    let body = async {
        let (tx, previous) = transaction(&fixture.pool).await?;
        let result = tx
            .application(async |sql| {
                let _ = sqlx::query("SELECT 1 / 0").execute(sql.executor()).await;
                Ok::<_, sqlx::Error>(())
            })
            .await;
        assert!(matches!(
            result,
            Err(PgScopeError::Transaction(PgTransactionError::Query(_)))
        ));
        replacement(&fixture.pool, previous).await?;
        let (tx, previous) = transaction(&fixture.pool).await?;
        let error = tx
            .application(async |_| Err::<(), _>(std::io::Error::other("private message")))
            .await
            .unwrap_err();
        assert!(matches!(error, PgScopeError::Application(_)));
        assert!(!format!("{error:?}: {error}").contains("private message"));
        replacement(&fixture.pool, previous).await?;
        let (tx, previous) = transaction(&fixture.pool).await?;
        let error = tx
            .operation(async |sql| {
                sqlx::raw_sql("COMMIT").execute(sql.executor()).await?;
                Err::<(), _>(sqlx::Error::Protocol("original cause".into()))
            })
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            PgScopeError::Recovery {
                application: sqlx::Error::Protocol(_),
                ..
            }
        ));
        replacement(&fixture.pool, previous).await?;
        Ok(())
    }
    .await;
    fixture.finish(body).await
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn deferred_and_transport_commit_failures_are_unconfirmed() -> Result {
    let mut fixture = fixture().await?;
    let body = async {
        sqlx::raw_sql("CREATE TEMP TABLE atomic_deferred(id integer UNIQUE DEFERRABLE INITIALLY DEFERRED)")
            .execute(&fixture.pool).await?;
        let (tx, previous) = transaction(&fixture.pool).await?;
        let (tx, ()) = tx.application(async |sql| {
            sqlx::raw_sql("INSERT INTO atomic_deferred VALUES (1), (1)").execute(sql.executor()).await?;
            Ok::<_, sqlx::Error>(())
        }).await?;
        let failure = tx.commit().await.unwrap_err();
        assert!(matches!(failure.cause(), PgTransactionError::Query(error) if error.native().as_database_error().unwrap().code().as_deref() == Some("23505")));
        replacement(&fixture.pool, previous).await?;
        let (tx, previous) = transaction(&fixture.pool).await?;
        sqlx::query("SELECT pg_terminate_backend($1)").bind(previous).execute(&mut fixture.observer).await?;
        let _unconfirmed = tx.commit().await.unwrap_err();
        replacement(&fixture.pool, previous).await?;
        Ok(())
    }.await;
    fixture.finish(body).await
}
