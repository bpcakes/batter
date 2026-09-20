use super::*;
use batter_sqlx::PgReadOnlySnapshot;

#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn atomic_and_snapshot_completion_retire_session_state() -> Result {
    let mut fixture = fixture().await?;
    let key = fixture.key + 1;
    let body = async {
        for mode in 0..3 {
            let previous = if mode == 2 {
                PgReadOnlySnapshot::inspect(&fixture.pool, async |sql| {
                    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
                        .fetch_one(sql.executor()).await?;
                    sqlx::query("SELECT pg_advisory_lock($1)").bind(key).execute(sql.executor()).await?;
                    Ok::<_, sqlx::Error>(pid)
                }).await?
            } else {
                let (tx, previous) = transaction(&fixture.pool).await?;
                let (tx, ()) = tx.application(async |sql| {
                    sqlx::raw_sql("SET search_path = hidden_application_schema; SET statement_timeout = '7s'")
                        .execute(sql.executor()).await?;
                    sqlx::query("SELECT pg_advisory_lock($1)").bind(key).execute(sql.executor()).await?;
                    Ok::<_, sqlx::Error>(())
                }).await?;
                if mode == 0 { let _ = tx.commit().await?; }
                else { let _ = tx.rollback().await?; }
                previous
            };
            replacement(&fixture.pool, previous).await?;
            let (path, timeout): (String, String) = sqlx::query_as("SELECT current_setting('search_path'), current_setting('statement_timeout')")
                .fetch_one(&fixture.pool).await?;
            assert!(!path.contains("hidden_application_schema"));
            assert_eq!(timeout, "0");
            // Retirement is local; independently await eventual server release.
            tokio::time::timeout(support::OBSERVE, async {
                loop {
                    let acquired: bool = sqlx::query_scalar("SELECT pg_try_advisory_lock($1)")
                        .bind(key).fetch_one(&mut fixture.observer).await?;
                    if acquired { break Ok::<_, sqlx::Error>(()); }
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
            }).await??;
            sqlx::query("SELECT pg_advisory_unlock($1)").bind(key).execute(&mut fixture.observer).await?;
        }
        Ok(())
    }.await;
    fixture.finish(body).await
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn acquisition_resets_inherited_state_and_statement_cache() -> Result {
    let mut fixture = fixture().await?;
    let key = fixture.key + 1;
    let body = async {
        // Populate SQLx's statement cache before changing session state.
        let previous: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&fixture.pool).await?;
        sqlx::raw_sql("SET search_path = inherited_shadow; CREATE TEMP TABLE inherited_temp(id integer)")
            .execute(&fixture.pool).await?;
        sqlx::query("SELECT pg_advisory_lock($1)").bind(key).execute(&fixture.pool).await?;
        let tx = PgAtomicTransaction::begin(&fixture.pool).await?;
        let (tx, ()) = tx.application(async |sql| {
            assert_eq!(pid(sql).await?, previous, "reset checks the inherited physical session");
            let (path, temp): (String, Option<String>) = sqlx::query_as("SELECT current_setting('search_path'), to_regclass('pg_temp.inherited_temp')::text")
                .fetch_one(sql.executor()).await?;
            assert!(!path.contains("inherited_shadow"));
            assert_eq!(temp, None);
            Ok::<_, sqlx::Error>(())
        }).await?;
        let acquired: bool = sqlx::query_scalar("SELECT pg_try_advisory_lock($1)")
            .bind(key).fetch_one(&mut fixture.observer).await?;
        assert!(acquired, "begin must release inherited session locks");
        sqlx::query("SELECT pg_advisory_unlock($1)").bind(key).execute(&mut fixture.observer).await?;
        let _ = tx.rollback().await?;
        Ok(())
    }.await;
    fixture.finish(body).await
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn failed_session_normalization_retires_instead_of_returning() -> Result {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    let fixture = fixture().await?;
    let body = async {
        let terminate = Arc::new(AtomicBool::new(false));
        let hook = terminate.clone();
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .before_acquire(move |connection, _| {
                let terminate = hook.swap(false, Ordering::SeqCst);
                Box::pin(async move {
                    if terminate {
                        // Pool liveness has already been checked. Hand the owner
                        // a dead session so its first normalization round trip
                        // fails, rather than testing an acquisition failure.
                        sqlx::query("SELECT pg_terminate_backend(pg_backend_pid())")
                            .execute(connection)
                            .await
                            .expect_err("terminate this backend");
                    }
                    Ok(true)
                })
            })
            .connect_with((*fixture.pool.connect_options()).clone())
            .await?;
        let previous: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&pool)
            .await?;
        terminate.store(true, Ordering::SeqCst);
        assert!(PgAtomicTransaction::begin(&pool).await.is_err());
        replacement(&pool, previous).await?;
        support::bounded(pool.close()).await?;
        Ok(())
    }
    .await;
    fixture.finish(body).await
}
