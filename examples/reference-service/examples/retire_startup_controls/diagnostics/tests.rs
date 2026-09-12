use super::*;
use batter::{cleanup::CleanupBudget, command::Command, operation::OperationContext};
use batter_example_reference_service::retirement::CancellationFailure;
use sqlx::types::Uuid;
use std::time::Duration;

async fn execute(
    work: Result<RetirementReport, RetirementError>,
    cleanup_fails: bool,
) -> (Value, bool) {
    let second = Duration::from_secs(1);
    let command = Command::new(
        OperationContext::new(second * 10).unwrap(),
        CleanupBudget::new(second, second, second).unwrap(),
        move |scope| {
            Box::pin(async move {
                scope.stage("retirement.cancel")?;
                scope.reserve_cleanup("pool")?.register(move || async move {
                    if cleanup_fails {
                        return Err(std::io::Error::other("private cleanup detail").into());
                    }
                    Ok(())
                });
                work
            })
        },
    )
    .start();
    outcome(command.wait().await)
}

#[tokio::test]
async fn cancellation_readback_key_survives_session_wrapping_and_failed_cleanup() {
    let job = Uuid::from_u128(42);
    let error = RetirementError::SessionReplaced {
        original: Some(Box::new(RetirementError::Cancellation(Box::new(
            CancellationFailure {
                job_id: job,
                cancelled_before_failure: 3,
                native: runledger_postgres::Error::ConnectionError("private native detail".into()),
            },
        )))),
    };
    let (facts, success) = execute(Err(error), true).await;
    assert!(!success);
    assert_eq!(facts["status"], "incomplete");
    assert_eq!(facts["stage"], "retirement.cancel");
    assert_eq!(facts["work"]["kind"], "session_replaced");
    assert_eq!(facts["work"]["original"]["job_id"], job.to_string());
    assert_eq!(facts["work"]["original"]["cancelled_before_failure"], 3);
    assert_eq!(facts["work"]["original"]["readback_attempted"], false);
    assert_eq!(facts["cleanup"]["succeeded"], false);
    assert_eq!(facts["cleanup"]["report_available"], true);
    assert_eq!(facts["cleanup"]["attempted"], 1);
    assert_eq!(facts["retry_authorized"], false);
    assert!(!facts.to_string().contains("private"));
}

#[tokio::test]
async fn identity_and_quiescence_refusals_expose_structured_facts() {
    let (facts, success) = execute(
        Err(RetirementError::WrongDatabase {
            actual: DatabaseIdentity::new(-42, 17).unwrap(),
        }),
        false,
    )
    .await;
    assert!(!success);
    assert_eq!(facts["work"]["kind"], "wrong_database");
    assert_eq!(facts["work"]["actual"]["system_identifier"], -42);
    assert_eq!(facts["work"]["actual"]["database_oid"], 17);
    assert_eq!(facts["cleanup"]["succeeded"], true);
    let (facts, success) = execute(
        Err(RetirementError::NotOffline {
            other_backends: 2,
            prepared_transactions: 1,
            cancelled: 5,
        }),
        false,
    )
    .await;
    assert!(!success);
    assert_eq!(facts["work"]["other_backends"], 2);
    assert_eq!(facts["work"]["prepared_transactions"], 1);
    assert_eq!(facts["work"]["cancelled"], 5);
}

#[tokio::test]
async fn completed_work_still_exposes_failed_cleanup() {
    for cleanup_fails in [false, true] {
        let (facts, success) = execute(
            Ok(RetirementReport {
                identity: DatabaseIdentity::new(1, 2).unwrap(),
                definition: DefinitionState::Absent,
                cancelled: 7,
            }),
            cleanup_fails,
        )
        .await;
        assert_eq!(success, !cleanup_fails);
        assert_eq!(facts["work"]["kind"], "completed");
        assert_eq!(facts["work"]["definition"], "absent");
        assert_eq!(facts["work"]["cancelled"], 7);
        assert_eq!(facts["cleanup"]["succeeded"], !cleanup_fails);
        assert_eq!(facts["deployment_state_inspected"], false);
        assert!(!facts.to_string().contains("private"));
    }
}

#[tokio::test]
async fn real_begin_failure_preserves_transaction_classification() {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy_with(sqlx::postgres::PgConnectOptions::new());
    pool.close().await;
    let job = Uuid::from_u128(77);
    let native = runledger_postgres::jobs::cancel_job_with_scope(
        &pool,
        runledger_postgres::jobs::JobCancellationScope::Global,
        job,
        None,
    )
    .await
    .unwrap_err();
    let (facts, success) = execute(
        Err(RetirementError::Cancellation(Box::new(
            CancellationFailure {
                job_id: job,
                cancelled_before_failure: 0,
                native,
            },
        ))),
        false,
    )
    .await;
    assert!(!success);
    assert_eq!(
        facts["work"]["native"]["code"],
        "db.transaction_begin_failed"
    );
    assert_eq!(facts["work"]["native"]["category"], "internal");
    assert_eq!(facts["work"]["job_id"], job.to_string());
    assert_eq!(facts["work"]["readback_attempted"], false);
}
