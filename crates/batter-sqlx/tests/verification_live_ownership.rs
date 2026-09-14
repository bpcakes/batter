use super::support::{Result, require};
use super::{
    AuthorityFixture, Names, VerificationPolicy, names_policy, quote, verify_combined as verify,
};
use batter::operation::{Interruption, OperationContext, OperationError};
use batter_sqlx::verification::RolePolicy;
use std::time::Duration;

fn policy(names: &Names) -> Result<VerificationPolicy> {
    names_policy(
        names,
        &names.schema_a,
        &names.ledger_a,
        &names.table_a,
        RolePolicy::default(),
        Vec::new(),
        false,
        true,
        false,
        false,
        false,
    )
}

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn verification_preserves_separate_caller_raw_transactions() -> Result {
    AuthorityFixture::create()
        .await?
        .run(|fixture| {
            Box::pin(async move {
                let names = fixture.names.clone();
                let pool = fixture.login(&names.login_a).await?;
                let table = format!("{}.{}", quote(&names.schema_a), quote(&names.table_a));
                sqlx::query("BEGIN").execute(&mut fixture.admin).await?;
                sqlx::query(sqlx::AssertSqlSafe(format!(
                    "INSERT INTO {table} (id, visible, secret) VALUES (-11, 'pending', 'pending')"
                )))
                .execute(&mut fixture.admin)
                .await?;
                super::verify_policy(&pool, &policy(&names)?).await?;
                let retained: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
                    "SELECT count(*) FROM {table} WHERE id = -11"
                )))
                .fetch_one(&mut fixture.admin)
                .await?;
                let external: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
                    "SELECT count(*) FROM {table} WHERE id = -11"
                )))
                .fetch_one(&pool)
                .await?;
                require(
                    retained == 1 && external == 0,
                    "verification changed the separate caller's pending transaction",
                )?;
                sqlx::query("COMMIT").execute(&mut fixture.admin).await?;
                let committed: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
                    "SELECT count(*) FROM {table} WHERE id = -11"
                )))
                .fetch_one(&pool)
                .await?;
                require(
                    committed == 1,
                    "the caller transaction could not commit after verification",
                )?;
                pool.close().await;
                Ok(())
            })
        })
        .await
}

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn verification_occupied_pool_acquisition_preserves_caller_transaction() -> Result {
    AuthorityFixture::create()
        .await?
        .run(|fixture| {
            Box::pin(async move {
                let names = fixture.names.clone();
                let pool = fixture.login(&names.login_a).await?;
                let mut caller = pool.acquire().await?;
                let table = format!("{}.{}", quote(&names.schema_a), quote(&names.table_a));
                sqlx::query("BEGIN").execute(&mut *caller).await?;
                sqlx::query(sqlx::AssertSqlSafe(format!(
                    "INSERT INTO {table} (id, visible, secret) VALUES (-12, 'pending', 'pending')"
                )))
                .execute(&mut *caller)
                .await?;
                let context = OperationContext::new(Duration::from_millis(100))?;
                let result = verify(&pool, &context, &policy(&names)?).await;
                require(
                    matches!(
                        result,
                        Err(OperationError::Interrupted(Interruption::DeadlineExceeded))
                    ),
                    "occupied one-slot verifier acquisition did not obey its operation deadline",
                )?;
                let retained: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
                    "SELECT count(*) FROM {table} WHERE id = -12"
                )))
                .fetch_one(&mut *caller)
                .await?;
                require(
                    retained == 1,
                    "timed-out verification damaged the checked-out caller transaction",
                )?;
                sqlx::query("ROLLBACK").execute(&mut *caller).await?;
                drop(caller);
                super::verify_policy(&pool, &policy(&names)?).await?;
                pool.close().await;
                Ok(())
            })
        })
        .await
}

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn verification_normalizes_abandoned_pooled_raw_transactions() -> Result {
    AuthorityFixture::create()
        .await?
        .run(|fixture| {
            Box::pin(async move {
                let names = fixture.names.clone();
                let pool = fixture.login(&names.login_a).await?;
                let table = format!("{}.{}", quote(&names.schema_a), quote(&names.table_a));
                let mut abandoned = pool.acquire().await?;
                sqlx::query("BEGIN").execute(&mut *abandoned).await?;
                sqlx::query(sqlx::AssertSqlSafe(format!(
                    "INSERT INTO {table} (id, visible, secret) VALUES (-13, 'pending', 'pending')"
                )))
                .execute(&mut *abandoned)
                .await?;
                drop(abandoned);
                super::verify_policy(&pool, &policy(&names)?).await?;
                let count: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
                    "SELECT count(*) FROM {table} WHERE id = -13"
                )))
                .fetch_one(&pool)
                .await?;
                require(
                    count == 0,
                    "verification retained or committed an abandoned raw transaction",
                )?;
                for aborted in [false, true] {
                    let mut abandoned = pool.acquire().await?;
                    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
                        .fetch_one(&mut *abandoned).await?;
                    sqlx::query("BEGIN").execute(&mut *abandoned).await?;
                    if aborted {
                        let failure = sqlx::query("SELECT 1 / 0").execute(&mut *abandoned).await;
                        require(failure.as_ref().err().and_then(sqlx::Error::as_database_error)
                            .and_then(|error| error.code()).as_deref() == Some("22012"),
                            "the aborted-transaction control did not fail with division by zero")?;
                    }
                    drop(abandoned);
                    super::verify_policy(&pool, &policy(&names)?).await?;
                    let idle: bool = sqlx::query_scalar(
                        "SELECT state = 'idle' AND xact_start IS NULL FROM pg_stat_activity WHERE pid = $1"
                    ).bind(pid).fetch_one(&mut fixture.admin).await?;
                    require(idle, "verification did not normalize empty or aborted raw transaction state")?;
                }
                pool.close().await;
                Ok(())
            })
        })
        .await
}
