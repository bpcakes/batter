use super::{fixture, profile_timeouts::complete, replacement, support::Result};
use batter_sqlx::{
    PgAtomicError, PgAtomicUncertainty, PgReadOnlySnapshot, PgScopeError, PgScopeFailure,
    PgScopeLoss, PgSnapshotError, PgTransactionError, run_atomic_profiled,
};
use std::{sync::Arc, time::Duration};

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn transaction_timeout_drift_poison_retains_cause_and_retires() -> Result {
    let fixture = fixture().await?;
    let body = async {
        let login = fixture.pool.connect_options().get_username().to_owned();
        let profile = complete(&login, Duration::from_secs(11), Duration::from_secs(22));
        for name in ["idle_in_transaction_session_timeout", "transaction_timeout"] {
            let mut original = None;
            let mut previous = None;
            let error = run_atomic_profiled(&fixture.pool, &profile, async |scope| {
                let failure = scope
                    .application(async |sql| {
                        previous = Some(super::pid(sql).await?);
                        sqlx::query("SELECT set_config($1, '0', false)")
                            .bind(name)
                            .execute(sql.executor())
                            .await?;
                        Ok::<_, sqlx::Error>(())
                    })
                    .await
                    .expect_err("timeout drift must poison the scope");
                let PgScopeError::Terminal(PgScopeFailure::Transaction(first)) = failure else {
                    panic!("expected terminal profile mismatch");
                };
                assert!(matches!(*first, PgTransactionError::Query(_)));
                let next = scope
                    .application(async |_| {
                        panic!("poisoned scope must not invoke work");
                        #[allow(unreachable_code)]
                        Ok::<(), ()>(())
                    })
                    .await
                    .unwrap_err();
                let PgScopeError::Terminal(PgScopeFailure::Transaction(next)) = next else {
                    panic!("lost original terminal classification");
                };
                assert!(Arc::ptr_eq(&first, &next));
                original = Some(first);
                Ok::<_, ()>(42)
            })
            .await
            .expect_err("caught drift cannot commit");
            let PgAtomicError::Uncertain(PgAtomicUncertainty::ScopeLost {
                result: Ok(42),
                cause: PgScopeLoss::Transaction(cause),
            }) = error
            else {
                panic!("lost output or terminal cause");
            };
            assert!(Arc::ptr_eq(&cause, original.as_ref().unwrap()));
            replacement(&fixture.pool, previous.unwrap()).await?;
        }
        Ok(())
    }
    .await;
    fixture.finish(body).await
}

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn snapshot_timeout_drift_rejects_success_and_preserves_recovered_errors() -> Result {
    let fixture = fixture().await?;
    let body = async {
        let login = fixture.pool.connect_options().get_username().to_owned();
        let profile = complete(&login, Duration::from_secs(11), Duration::from_secs(22));
        for name in ["idle_in_transaction_session_timeout", "transaction_timeout"] {
            let mut previous = None;
            let error =
                PgReadOnlySnapshot::inspect_profiled(&fixture.pool, &profile, async |sql| {
                    previous = Some(
                        sqlx::query_scalar::<_, i32>("SELECT pg_backend_pid()")
                            .fetch_one(sql.executor())
                            .await?,
                    );
                    sqlx::query("SELECT set_config($1, '0', false)")
                        .bind(name)
                        .execute(sql.executor())
                        .await?;
                    Ok::<_, sqlx::Error>(42)
                })
                .await
                .expect_err("snapshot cannot return evidence after drift");
            assert!(matches!(
                error,
                PgSnapshotError::Transaction(PgTransactionError::Query(_))
            ));
            replacement(&fixture.pool, previous.unwrap()).await?;
            // Rollback to the private guard also rolls back GUC changes. A
            // recovered inspection error must retain its original classification.
            let error =
                PgReadOnlySnapshot::inspect_profiled(&fixture.pool, &profile, async |sql| {
                    sqlx::query("SELECT set_config($1, '0', false)")
                        .bind(name)
                        .execute(sql.executor())
                        .await?;
                    Err::<(), _>(sqlx::Error::Protocol("private inspection error".into()))
                })
                .await
                .unwrap_err();
            assert!(
                matches!(&error, PgSnapshotError::Inspection(sqlx::Error::Protocol(message))
                if message == "private inspection error")
            );
            assert!(!format!("{error} {error:?}").contains("private"));
        }
        Ok(())
    }
    .await;
    fixture.finish(body).await
}
