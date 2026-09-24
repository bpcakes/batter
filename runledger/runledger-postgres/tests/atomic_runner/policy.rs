use super::*;
use runledger_postgres::{
    PgAtomicUncertainty, PgFailurePolicy, PgScopeFailure, PgScopeLoss, PgScopeRolledBack,
    PgTransactionError, RequiredIntentError, run_atomic_fail_fast_with, run_atomic_with,
};
use std::sync::Arc;

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
        check_intent_queries(&mut scope).await?;
        scope.record_required_job_enqueue_intent(&intent).await?;
        let mut queue = scope.queue();
        queue.enqueue_job(&request).await?;
        check_queue_queries(&mut queue).await?;
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

async fn check_intent_queries(
    scope: &mut runledger_postgres::PgPolicyIntentScope<'_, (), Policy>,
) -> Result<(), ConsumerError> {
    scope
        .execute(sqlx::query("INSERT INTO policy_audit VALUES (1)"))
        .await?;
    assert_eq!(
        scope
            .fetch_one(sqlx::query_scalar::<_, i64>("SELECT 42::bigint"))
            .await?,
        42
    );
    assert!(
        scope
            .fetch_optional(sqlx::query_scalar::<_, i64>(
                "SELECT 42::bigint WHERE false"
            ))
            .await?
            .is_none()
    );
    assert_eq!(
        scope
            .fetch_all(sqlx::query_scalar::<_, i64>("SELECT 42::bigint"))
            .await?,
        vec![42]
    );
    Ok(())
}

async fn check_queue_queries(
    scope: &mut runledger_postgres::PgPolicyQueueScope<'_, (), Policy>,
) -> Result<(), ConsumerError> {
    scope.execute(sqlx::query("SELECT 1")).await?;
    assert_eq!(
        scope
            .fetch_one(sqlx::query_scalar::<_, i64>("SELECT 42::bigint"))
            .await?,
        42
    );
    assert!(
        scope
            .fetch_optional(sqlx::query_scalar::<_, i64>(
                "SELECT 42::bigint WHERE false"
            ))
            .await?
            .is_none()
    );
    assert_eq!(
        scope
            .fetch_all(sqlx::query_scalar::<_, i64>("SELECT 42::bigint"))
            .await?,
        vec![42]
    );
    Ok(())
}

fn assert_native_check_failure(error: ConsumerError, constraint: &str) {
    let ConsumerError::Native(runledger_postgres::Error::QueryError(error)) = error else {
        panic!("native storage error must reach the consumer through From<Error>")
    };
    assert_eq!(error.sqlstate(), Some("23514"));
    assert_eq!(error.constraint(), Some(constraint));
    let source = error.source_arc().expect("retained SQLx cause");
    let native = source.as_database_error().expect("native database error");
    assert_eq!(native.code().as_deref(), Some("23514"));
    assert_eq!(native.constraint(), Some(constraint));
}

fn assert_required_check_failure(error: ConsumerError, constraint: &str) {
    let ConsumerError::Required(RequiredIntentError::Storage(
        runledger_postgres::Error::QueryError(error),
    )) = error
    else {
        panic!("required intent storage error must retain its distinct conversion")
    };
    assert_eq!(error.sqlstate(), Some("23514"));
    assert_eq!(error.constraint(), Some(constraint));
    let source = error.source_arc().expect("retained SQLx cause");
    let native = source.as_database_error().expect("native database error");
    assert_eq!(native.code().as_deref(), Some("23514"));
    assert_eq!(native.constraint(), Some(constraint));
}

#[tokio::test]
async fn required_intent_storage_failure_recovers_before_later_sql() {
    let (pool, database) = setup_ephemeral_pool("atomic_required_storage", 4).await;
    let profiled = support::profiled_database(&pool).await;
    sqlx::raw_sql(
        "CREATE TABLE policy_required_audit (id integer);
         ALTER TABLE job_enqueue_intents ADD CONSTRAINT policy_required_rejected
             CHECK (idempotency_key <> 'rejected-required')",
    )
    .execute(&pool)
    .await
    .expect("create required-intent failure control");
    let payload = serde_json::json!({"request": 1});
    let rejected = JobEnqueueIntent::new(
        JobType::new("test.atomic.required"),
        &payload,
        "rejected-required",
    );
    let accepted = JobEnqueueIntent::new(
        JobType::new("test.atomic.required"),
        &payload,
        "accepted-required",
    );

    run_atomic_with(&profiled, &Policy, async |mut scope| {
        let error = scope
            .record_required_job_enqueue_intent(&rejected)
            .await
            .expect_err("constraint must reject required intent");
        assert_required_check_failure(error, "policy_required_rejected");
        scope
            .sql(async |sql| {
                sqlx::query("INSERT INTO policy_required_audit VALUES (1)")
                    .execute(sql.executor())
                    .await?;
                Ok(())
            })
            .await?;
        scope.record_required_job_enqueue_intent(&accepted).await?;
        Ok(())
    })
    .await
    .expect("recovered required-intent error permits acknowledged commit");

    let audit: Vec<i32> = sqlx::query_scalar("SELECT id FROM policy_required_audit")
        .fetch_all(&pool)
        .await
        .expect("read committed audit");
    assert_eq!(audit, [1]);
    let keys: Vec<String> = sqlx::query_scalar(
        "SELECT idempotency_key FROM job_enqueue_intents ORDER BY idempotency_key",
    )
    .fetch_all(&pool)
    .await
    .expect("read committed intents");
    assert_eq!(keys, ["accepted-required"]);
    profiled.pool().close().await;
    teardown_ephemeral_pool(pool, database).await;
}

#[tokio::test]
async fn named_required_intent_observes_terminal_scope_loss() {
    let (pool, database) = setup_ephemeral_pool("atomic_named_loss", 4).await;
    let profiled = support::profiled_database(&pool).await;
    sqlx::query("CREATE TABLE policy_loss_audit (id integer)")
        .execute(&pool)
        .await
        .expect("create rollback evidence table");
    let payload = serde_json::json!({"request": 1});
    let intent = JobEnqueueIntent::new(
        JobType::new("test.atomic.named.loss"),
        &payload,
        "named-loss",
    );
    let mut observed = None;
    let result = run_atomic_with(&profiled, &Policy, async |mut scope| {
        let backend: i32 = scope
            .sql(async |sql| {
                sqlx::query("INSERT INTO policy_loss_audit VALUES (1)")
                    .execute(sql.executor())
                    .await?;
                sqlx::query_scalar("SELECT pg_backend_pid()")
                    .fetch_one(sql.executor())
                    .await
                    .map_err(Into::into)
            })
            .await?;
        let terminated: bool = sqlx::query_scalar("SELECT pg_terminate_backend($1)")
            .bind(backend)
            .fetch_one(&pool)
            .await
            .expect("terminate transaction backend from separate connection");
        assert!(terminated);
        let error = scope
            .record_required_job_enqueue_intent(&intent)
            .await
            .expect_err("named operation must observe terminal loss");
        let ConsumerError::Scope(failure) = error else {
            panic!("named operation must map loss through policy")
        };
        let cause = match *failure {
            PgScopeFailure::Transaction(cause)
            | PgScopeFailure::Recovery {
                recovery: cause, ..
            } => cause,
            PgScopeFailure::OperationAbandoned => {
                panic!("completed named call cannot be an abandoned operation")
            }
        };
        observed = Some(cause);
        Ok(())
    })
    .await;
    let Err(ConsumerError::Completion(outcome)) = result else {
        panic!("caught named-operation loss cannot commit")
    };
    let PgAtomicUncertainty::ScopeLost {
        result: Ok(()),
        cause: PgScopeLoss::Transaction(cause),
    } = *outcome
    else {
        panic!("outer result must retain terminal loss and provisional output")
    };
    assert!(Arc::ptr_eq(
        &cause,
        observed.as_ref().expect("inner loss cause")
    ));
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM policy_loss_audit")
        .fetch_one(&pool)
        .await
        .expect("read rolled-back audit count");
    assert_eq!(count, 0);
    profiled.pool().close().await;
    teardown_ephemeral_pool(pool, database).await;
}

#[tokio::test]
async fn observation_and_resource_enqueue_preserve_native_errors_and_recover() {
    use runledger_postgres::jobs::{JobEnqueueIntentDisposition, JobEnqueueIntentOutcomeState};

    let (pool, database) = setup_ephemeral_pool("atomic_policy_recovery", 4).await;
    let profiled = support::profiled_database(&pool).await;
    support::register_test_job_definition(&pool, "test.atomic.policy.recovery").await;
    // Fail inside each named operation's SQL, so continuing requires actual
    // savepoint recovery rather than only handling a validation rejection.
    sqlx::raw_sql(
        "CREATE TABLE policy_recovery_audit (id integer);
         ALTER TABLE job_enqueue_intents ADD CONSTRAINT policy_intent_rejected
             CHECK (idempotency_key <> 'rejected-intent');
         ALTER TABLE job_queue ADD CONSTRAINT policy_resource_rejected
             CHECK (execution_resource_key <> 'resource:rejected')",
    )
    .execute(&pool)
    .await
    .expect("create recovery controls");
    let payload = serde_json::json!({"request": 1});
    let rejected_intent = JobEnqueueIntent::new(
        JobType::new("test.atomic.policy.recovery"),
        &payload,
        "rejected-intent",
    );
    let accepted_intent = JobEnqueueIntent::new(
        JobType::new("test.atomic.policy.recovery"),
        &payload,
        "accepted-intent",
    );
    let request = JobEnqueue {
        job_type: JobType::new("test.atomic.policy.recovery"),
        organization_id: None,
        payload: &payload,
        priority: None,
        max_attempts: None,
        timeout_seconds: None,
        next_run_at: None,
        idempotency_key: Some("policy-recovery"),
        stage: None,
    };

    run_atomic_with(&profiled, &Policy, async |mut scope| {
        let error = scope
            .observe_job_enqueue_intent(&rejected_intent)
            .await
            .expect_err("intent check must fail");
        assert_native_check_failure(error, "policy_intent_rejected");
        scope
            .sql(async |sql| {
                sqlx::query("INSERT INTO policy_recovery_audit VALUES (1)")
                    .execute(sql.executor())
                    .await?;
                Ok(())
            })
            .await?;
        let observed = scope.observe_job_enqueue_intent(&accepted_intent).await?;
        assert_eq!(observed.disposition, JobEnqueueIntentDisposition::Inserted);
        assert_eq!(observed.state, JobEnqueueIntentOutcomeState::Pending);

        let mut queue = scope.queue();
        let error = queue
            .enqueue_job_with_execution_resource(&request, "resource:rejected")
            .await
            .expect_err("resource check must fail");
        assert_native_check_failure(error, "policy_resource_rejected");
        queue
            .sql(async |sql| {
                sqlx::query("INSERT INTO policy_recovery_audit VALUES (2)")
                    .execute(sql.executor())
                    .await?;
                Ok(())
            })
            .await?;
        queue
            .enqueue_job_with_execution_resource(&request, "resource:accepted")
            .await?;
        Ok(())
    })
    .await
    .expect("caught storage failures leave the transaction usable");

    let audit: Vec<i32> = sqlx::query_scalar("SELECT id FROM policy_recovery_audit ORDER BY id")
        .fetch_all(&pool)
        .await
        .expect("read committed recovery writes");
    assert_eq!(audit, [1, 2]);
    let intents: Vec<String> =
        sqlx::query_scalar("SELECT idempotency_key FROM job_enqueue_intents")
            .fetch_all(&pool)
            .await
            .expect("read committed intent");
    assert_eq!(intents, ["accepted-intent"]);
    let jobs: Vec<(String, String)> =
        sqlx::query_as("SELECT idempotency_key, execution_resource_key FROM job_queue")
            .fetch_all(&pool)
            .await
            .expect("read committed resource enqueue");
    assert_eq!(
        jobs,
        [("policy-recovery".into(), "resource:accepted".into())]
    );
    profiled.pool().close().await;
    teardown_ephemeral_pool(pool, database).await;
}
