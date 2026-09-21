use super::*;
use batter_sqlx::{
    PgAtomicError, PgAtomicUncertainty, PgScopeError, PgScopeFailure, PgScopeLoss,
    PgTransactionError, run_atomic,
};
use std::{error::Error, sync::Arc};

#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn caught_boundary_loss_retains_first_cause() -> Result {
    let fixture = fixture().await?;
    let body = async {
        let mut original = None;
        let error = run_atomic(&fixture.pool, async |scope| {
            let failure = scope
                .application(async |sql| {
                    sqlx::raw_sql("ROLLBACK").execute(sql.executor()).await?;
                    Ok::<_, sqlx::Error>(())
                })
                .await
                .expect_err("raw rollback loses boundary");
            let PgScopeError::Terminal(PgScopeFailure::Transaction(cause)) = failure else {
                panic!("expected terminal transaction failure");
            };
            assert!(matches!(
                *cause,
                PgTransactionError::TransactionBoundaryLost
            ));
            original = Some(cause);
            Ok::<_, std::io::Error>(42)
        })
        .await
        .expect_err("caught poison cannot commit");
        let source = error.source().expect("outer loss has source");
        let loss = source
            .downcast_ref::<PgScopeLoss>()
            .expect("explicit loss reason");
        assert!(matches!(loss, PgScopeLoss::Transaction(_)));
        assert!(matches!(
            source
                .source()
                .and_then(|error| error.downcast_ref::<PgTransactionError>()),
            Some(PgTransactionError::TransactionBoundaryLost)
        ));
        let PgAtomicError::Uncertain(PgAtomicUncertainty::ScopeLost {
            result: Ok(42),
            cause: PgScopeLoss::Transaction(cause),
        }) = error
        else {
            panic!("expected output and original poison");
        };
        assert!(Arc::ptr_eq(
            &cause,
            original.as_ref().expect("callback captured cause")
        ));
        Ok(())
    }
    .await;
    fixture.finish(body).await
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn caught_recovery_failure_retains_poison_and_callback_rejection() -> Result {
    let fixture = fixture().await?;
    let body = async {
        let mut original = None;
        let error = run_atomic(&fixture.pool, async |scope| {
            let failure = scope.application(async |sql| {
                sqlx::raw_sql("ROLLBACK").execute(sql.executor()).await?;
                Err::<(), _>(sqlx::Error::Protocol("private inner failure".into()))
            }).await.expect_err("guard no longer exists");
            let PgScopeError::Terminal(PgScopeFailure::Recovery { application, recovery }) = failure else {
                panic!("expected both body and recovery errors");
            };
            assert!(matches!(application, sqlx::Error::Protocol(ref text) if text == "private inner failure"));
            assert!(matches!(*recovery, PgTransactionError::Query(_)));
            original = Some(recovery);
            Err::<(), _>(std::io::Error::other("private outer rejection"))
        }).await.expect_err("cleanup failure cannot be acknowledged rollback");
        assert!(!format!("{error:?} {error}").contains("private"));
        let PgAtomicError::Uncertain(PgAtomicUncertainty::ScopeLost {
            result: Err(rejection), cause: PgScopeLoss::Transaction(cause),
        }) = error else { panic!("expected callback rejection and poison"); };
        assert_eq!(rejection.to_string(), "private outer rejection");
        assert!(Arc::ptr_eq(&cause, original.as_ref().expect("retained recovery error")));
        Ok(())
    }.await;
    fixture.finish(body).await
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn poisoned_scope_repeats_original_cause_without_invoking_work() -> Result {
    let fixture = fixture().await?;
    let body = async {
        let mut original = None;
        let error = run_atomic(&fixture.pool, async |scope| {
            let failure = scope
                .application(async |sql| {
                    // An aborted statement swallowed by the body fails validation.
                    // Its original native cause must not become BoundaryLost later.
                    let _ = sqlx::query("SELECT 1 / 0").execute(sql.executor()).await;
                    Ok::<_, ()>(())
                })
                .await
                .expect_err("aborted transaction fails validation");
            let PgScopeError::Terminal(PgScopeFailure::Transaction(first)) = failure else {
                panic!("expected native poison");
            };
            assert!(matches!(*first, PgTransactionError::Query(_)));
            for _ in 0..2 {
                let mut invoked = false;
                let failure = scope
                    .application(async |_| {
                        invoked = true;
                        Ok::<(), ()>(())
                    })
                    .await
                    .expect_err("poison remains terminal");
                assert!(!invoked, "poisoned scope invoked another operation");
                let PgScopeError::Terminal(PgScopeFailure::Transaction(next)) = failure else {
                    panic!("poison classification changed");
                };
                assert!(Arc::ptr_eq(&first, &next));
            }
            original = Some(first);
            Ok::<_, ()>(99)
        })
        .await
        .expect_err("poisoned scope cannot commit");
        let PgAtomicError::Uncertain(PgAtomicUncertainty::ScopeLost {
            result: Ok(99),
            cause: PgScopeLoss::Transaction(cause),
        }) = error
        else {
            panic!("expected first poison");
        };
        assert!(Arc::ptr_eq(&cause, original.as_ref().expect("first cause")));
        Ok(())
    }
    .await;
    fixture.finish(body).await
}
