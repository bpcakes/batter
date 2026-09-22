use super::*;
use runledger_postgres::{
    PgAtomicUncertainty, PgFailurePolicy, PgScopeFailure, PgScopeLoss, PgScopeRolledBack,
    PgTransactionError, RequiredIntentError, run_atomic_fail_fast_with, run_atomic_with,
};

#[derive(Debug)]
enum ConsumerError {
    Sql(sqlx::Error),
    RolledBack(PgScopeRolledBack),
    Native(runledger_postgres::Error),
    Required(RequiredIntentError),
    Begin(PgTransactionError),
    Scope(Box<PgScopeFailure<Self>>),
    Completion(Box<PgAtomicUncertainty<(), Self>>),
}
impl From<PgScopeRolledBack> for ConsumerError {
    fn from(value: PgScopeRolledBack) -> Self {
        Self::RolledBack(value)
    }
}
impl From<sqlx::Error> for ConsumerError {
    fn from(error: sqlx::Error) -> Self {
        Self::Sql(error)
    }
}
impl From<runledger_postgres::Error> for ConsumerError {
    fn from(error: runledger_postgres::Error) -> Self {
        Self::Native(error)
    }
}
impl From<RequiredIntentError> for ConsumerError {
    fn from(error: RequiredIntentError) -> Self {
        Self::Required(error)
    }
}
struct Policy;
impl PgFailurePolicy<()> for Policy {
    type Error = ConsumerError;
    fn begin_failed(&self, cause: PgTransactionError) -> ConsumerError {
        ConsumerError::Begin(cause)
    }
    fn scope_lost(&self, failure: PgScopeFailure<ConsumerError>) -> ConsumerError {
        ConsumerError::Scope(Box::new(failure))
    }
    fn commit_unconfirmed(&self, output: (), cause: PgTransactionError) -> ConsumerError {
        ConsumerError::Completion(Box::new(PgAtomicUncertainty::CommitUnconfirmed {
            output,
            cause,
        }))
    }
    fn rollback_unconfirmed(
        &self,
        rejection: ConsumerError,
        cause: PgTransactionError,
    ) -> ConsumerError {
        ConsumerError::Completion(Box::new(PgAtomicUncertainty::RollbackUnconfirmed {
            rejection,
            cause,
        }))
    }
    fn scope_lost_after_body(
        &self,
        result: Result<(), ConsumerError>,
        cause: PgScopeLoss,
    ) -> ConsumerError {
        ConsumerError::Completion(Box::new(PgAtomicUncertainty::ScopeLost { result, cause }))
    }
}

// Explicit inspection keeps native causes available; default formatting is not
// used as an error oracle. Most variants are exercised in the foundation suite.
impl ConsumerError {
    fn retained_cause(&self) -> &dyn std::fmt::Debug {
        match self {
            Self::Sql(cause) => cause,
            Self::RolledBack(cause) => cause,
            Self::Native(cause) => cause,
            Self::Required(cause) => cause,
            Self::Begin(cause) => cause,
            Self::Scope(cause) => cause,
            Self::Completion(cause) => cause,
        }
    }
}

#[tokio::test]
async fn one_policy_covers_sql_intent_queue_and_required_rejection() {
    for fast in [false, true] {
        exercise_policy(fast).await;
    }
}

async fn exercise_policy(fast: bool) {
    let (pool, database) = setup_ephemeral_pool("atomic_policy", 4).await;
    let profiled = support::profiled_database(&pool).await;
    support::register_test_job_definition(&pool, "test.atomic.policy").await;
    sqlx::query("CREATE TABLE policy_audit (id integer)")
        .execute(&pool)
        .await
        .expect("create application audit table");
    let payload = serde_json::json!({"request": 1});
    let intent = JobEnqueueIntent::new(JobType::new("test.atomic.policy"), &payload, "policy-key");
    let request = JobEnqueue {
        job_type: JobType::new("test.atomic.policy"),
        organization_id: None,
        payload: &payload,
        priority: None,
        max_attempts: None,
        timeout_seconds: None,
        next_run_at: None,
        idempotency_key: Some("policy-key"),
        stage: None,
    };
    let work = async |mut scope: runledger_postgres::PgPolicyIntentScope<'_, (), Policy>| {
        scope
            .sql(async |sql| {
                sqlx::query("INSERT INTO policy_audit VALUES (1)")
                    .execute(sql.executor())
                    .await?;
                Ok(())
            })
            .await?;
        scope.record_required_job_enqueue_intent(&intent).await?;
        let mut queue = scope.queue();
        queue.enqueue_job(&request).await?;
        queue
            .sql(async |sql| {
                sqlx::query("SELECT 1").execute(sql.executor()).await?;
                Ok(())
            })
            .await?;
        Ok(())
    };
    fn require_send<T: Send>(future: T) -> T {
        future
    }
    let result = if fast {
        require_send(run_atomic_fail_fast_with(&profiled, &Policy, work)).await
    } else {
        require_send(run_atomic_with(&profiled, &Policy, work)).await
    };
    result.unwrap_or_else(|error| panic!("policy workflow failed: {:?}", error.retained_cause()));
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM job_queue WHERE idempotency_key = 'policy-key'")
            .fetch_one(&pool)
            .await
            .expect("read committed queue row");
    assert_eq!(count, 1);

    // A required conflict uses the same consumer error as SQL, with no wrapping
    // or type annotation at any question-mark in the transaction body.
    sqlx::query("UPDATE job_enqueue_intents SET status = 'CONFLICTED', promotion_attempts = 1, last_attempted_at = now(), conflicted_at = now(), last_error_code = 'fixture_conflict', last_error_message = 'persisted conflict fixture' WHERE idempotency_key = 'policy-key'")
        .execute(&pool).await.expect("seed persisted conflict");
    let work = async |mut scope: runledger_postgres::PgPolicyIntentScope<'_, (), Policy>| {
        scope
            .sql(async |sql| {
                sqlx::query("INSERT INTO policy_audit VALUES (2)")
                    .execute(sql.executor())
                    .await?;
                Ok(())
            })
            .await?;
        scope.record_required_job_enqueue_intent(&intent).await?;
        Ok(())
    };
    let result = if fast {
        run_atomic_fail_fast_with(&profiled, &Policy, work).await
    } else {
        run_atomic_with(&profiled, &Policy, work).await
    };
    assert!(matches!(
        result,
        Err(ConsumerError::Required(RequiredIntentError::Conflict(_)))
    ));
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM policy_audit WHERE id = 2")
        .fetch_one(&pool)
        .await
        .expect("read rolled-back audit count");
    assert_eq!(count, 0);
    profiled.pool().close().await;
    teardown_ephemeral_pool(pool, database).await;
}
