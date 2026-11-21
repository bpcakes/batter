use super::*;
use batter_sqlx::PgCommitError;
use sqlx::{AssertSqlSafe, Connection, PgConnection, PgPool};
use std::time::Duration;
use tokio::time::Instant;

async fn inserted(pool: &PgPool, table: &str, id: i32) -> Result<(PgAtomicTransaction, i32)> {
    Ok(PgAtomicTransaction::begin(pool)
        .await?
        .application(async |sql| {
            // A disconnect check would abort the blocked COMMIT this test lets finish.
            sqlx::raw_sql(AssertSqlSafe(format!(
                "SET LOCAL client_connection_check_interval = 0; INSERT INTO {table} VALUES ({id})"
            )))
            .execute(sql.executor())
            .await?;
            pid(sql).await
        })
        .await?)
}

async fn committed(observer: &mut PgConnection, table: &str) -> Result<Vec<i32>> {
    Ok(support::bounded(
        sqlx::query_scalar(AssertSqlSafe(format!("SELECT id FROM {table} ORDER BY id")))
            .fetch_all(observer),
    )
    .await??)
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn commit_until_reports_only_a_sent_commit_as_uncertain() -> Result {
    let mut fixture = fixture().await?;
    let table = format!("batter_commit_until_{}", fixture.key);
    sqlx::raw_sql(AssertSqlSafe(format!(
        "CREATE TABLE {table}(id integer UNIQUE DEFERRABLE INITIALLY DEFERRED)"
    )))
    .execute(&mut fixture.observer)
    .await?;
    let body = async {
        let (tx, previous) = inserted(&fixture.pool, &table, 1).await?;
        let stopped = tx
            .commit_until(Instant::now() + support::LOCAL, std::future::ready(()))
            .await;
        assert!(matches!(stopped, Err(PgCommitError::Interrupted)));
        fixture.retired.push(previous);
        replacement(&fixture.pool, previous).await?;

        let (tx, previous) = inserted(&fixture.pool, &table, 2).await?;
        let expired = tx
            .commit_until(Instant::now(), std::future::pending())
            .await;
        assert!(matches!(expired, Err(PgCommitError::DeadlineBeforeCommit)));
        fixture.retired.push(previous);
        replacement(&fixture.pool, previous).await?;
        assert_eq!(
            committed(&mut fixture.observer, &table).await?,
            Vec::<i32>::new()
        );

        let (tx, _) = inserted(&fixture.pool, &table, 3).await?;
        let _confirmed = support::bounded(
            tx.commit_until(Instant::now() + support::LOCAL, std::future::pending()),
        )
        .await??;
        assert_eq!(committed(&mut fixture.observer, &table).await?, [3]);

        // The deferred unique check makes COMMIT wait for this uncommitted row.
        let mut blocker = PgConnection::connect(&std::env::var("DATABASE_URL")?).await?;
        let mut blocking = blocker.begin().await?;
        sqlx::raw_sql(AssertSqlSafe(format!("INSERT INTO {table} VALUES (4)")))
            .execute(&mut *blocking)
            .await?;
        let (tx, previous) = inserted(&fixture.pool, &table, 4).await?;
        fixture.retired.push(previous);
        let deadline = Instant::now() + Duration::from_secs(1);
        let task = tokio::spawn(tx.commit_until(deadline, std::future::pending()));
        fixture.blocked(previous).await?;
        let query: String = sqlx::query_scalar("SELECT query FROM pg_stat_activity WHERE pid=$1")
            .bind(previous)
            .fetch_one(&mut fixture.observer)
            .await?;
        assert_eq!(query, "COMMIT", "the deadline must expire after submission");
        let unacknowledged = support::bounded(task).await??;
        assert!(matches!(
            unacknowledged,
            Err(PgCommitError::AcknowledgementDeadline)
        ));
        replacement(&fixture.pool, previous).await?;
        blocking.rollback().await?;
        blocker.close().await?;
        let observed = Instant::now() + support::OBSERVE;
        loop {
            let rows = committed(&mut fixture.observer, &table).await?;
            if rows == [3, 4] {
                break;
            }
            support::require!(
                Instant::now() < observed,
                "the abandoned COMMIT did not commit after its blocker rolled back",
            )?;
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        Ok(())
    }
    .await;
    support::bounded(
        sqlx::raw_sql(AssertSqlSafe(format!("DROP TABLE {table}"))).execute(&mut fixture.observer),
    )
    .await??;
    fixture.finish(body).await
}

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
