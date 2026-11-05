//! Consumer reconciliation reads retained lineage without reading payload columns.
#[path = "intent_reconciliation/commit.rs"]
mod commit;
#[path = "intent_reconciliation/fixture.rs"]
mod fixture;
mod support;

use fixture::*;
use runledger_core::jobs::JobType;
use runledger_postgres::jobs::{JobEnqueueIntent, JobScope, promote_job_enqueue_intents_for_types};
use runledger_postgres::{IntentCorrelationState, IntentReconciliation, PgAtomicError, run_atomic};
use sqlx::{AssertSqlSafe, types::Uuid};
use std::time::Duration;

#[tokio::test]
async fn restricted_reconciliation_observes_own_writes_and_requires_acknowledged_commit() {
    let f = Fixture::new("reconcile_owned").await;
    let payload = serde_json::json!({"secret": "not exposed"});
    let request = JobEnqueueIntent::new(JobType::new(TYPE), &payload, "accepted");
    let accepted = run_atomic(&f.service, async |mut scope| {
        let accepted = scope
            .record_required_job_enqueue_intent(&request)
            .await
            .expect("complete fixture database operation");
        let found = retained(
            scope
                .reconcile_job_enqueue_intent(JobScope::Global, accepted.intent_id())
                .await
                .expect("complete fixture database operation"),
        );
        assert_eq!(found.id(), accepted.intent_id());
        assert_eq!(found.job_type().as_str(), TYPE);
        assert!(matches!(found.state(), IntentCorrelationState::Pending));
        assert_eq!(
            found.definition().expect("registered definition").version(),
            1
        );
        // A separate transaction must not see the uncommitted intent.
        assert!(matches!(
            read(&f.service, JobScope::Global, accepted.intent_id()).await,
            IntentReconciliation::NotFound
        ));
        let mut queue = scope.queue();
        assert!(matches!(
            retained(
                queue
                    .reconcile_job_enqueue_intent(JobScope::Global, accepted.intent_id())
                    .await
                    .expect("construct validated fixture input")
            )
            .state(),
            IntentCorrelationState::Pending
        ));
        Ok::<_, ()>(accepted)
    })
    .await
    .expect("complete fixture database operation");
    assert_eq!(
        retained(read(&f.service, JobScope::Global, accepted.intent_id()).await).id(),
        accepted.intent_id()
    );

    let mut rejected_id = Uuid::nil();
    let request = JobEnqueueIntent::new(JobType::new(TYPE), &payload, "rejected");
    let rejected = run_atomic(&f.service, async |mut scope| {
        let accepted = scope
            .record_required_job_enqueue_intent(&request)
            .await
            .expect("complete fixture database operation");
        rejected_id = accepted.intent_id();
        let _ = retained(
            scope
                .reconcile_job_enqueue_intent(JobScope::Global, rejected_id)
                .await
                .expect("complete fixture database operation"),
        );
        Err::<(), _>("business rejection")
    })
    .await;
    assert!(matches!(
        rejected,
        Err(PgAtomicError::Rejected("business rejection"))
    ));
    assert!(matches!(
        read(&f.service, JobScope::Global, rejected_id).await,
        IntentReconciliation::NotFound
    ));

    for query in [
        "SELECT payload FROM job_enqueue_intents",
        "SELECT last_error_message FROM job_enqueue_intents",
        "SELECT payload FROM job_queue",
        "SELECT checkpoint FROM job_queue",
        "SELECT last_error_message FROM job_queue",
    ] {
        let error = sqlx::query(AssertSqlSafe(query))
            .execute(f.service.pool())
            .await
            .expect_err("metadata role must be denied this operation");
        assert_eq!(
            error
                .as_database_error()
                .expect("PostgreSQL permission error")
                .code()
                .as_deref(),
            Some("42501")
        );
    }
    f.finish().await;
}

#[tokio::test]
async fn reconciliation_is_scoped_and_coherent_across_promotion_and_retention() {
    let f = Fixture::new("reconcile_lifecycle").await;
    let organization = Uuid::now_v7();
    let payload = serde_json::json!({"secret": "not exposed"});
    let request = JobEnqueueIntent::new(JobType::new(TYPE), &payload, "lifecycle")
        .with_organization_id(organization);
    let accepted = run_atomic(&f.service, async |mut scope| {
        scope.record_required_job_enqueue_intent(&request).await
    })
    .await
    .expect("complete fixture database operation");
    let id = accepted.intent_id();
    for wrong in [JobScope::Global, JobScope::Organization(Uuid::now_v7())] {
        assert!(matches!(
            read(&f.service, wrong, id).await,
            IntentReconciliation::NotFound
        ));
    }
    let scope = JobScope::Organization(organization);
    assert_eq!(retained(read(&f.service, scope, id).await).scope(), scope);
    let mut definition = f
        .pool
        .begin()
        .await
        .expect("begin concurrent fixture transaction");
    sqlx::query("UPDATE job_definitions SET version=2 WHERE job_type=$1")
        .bind(TYPE)
        .execute(&mut *definition)
        .await
        .expect("complete fixture database operation");
    let promoter = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect_with((*f.pool.connect_options()).clone())
        .await
        .expect("complete fixture database operation");
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&promoter)
        .await
        .expect("complete fixture database operation");
    let types = [JobType::new(TYPE)];
    let promoting = promote_job_enqueue_intents_for_types(&promoter, &types, 10);
    let observe = async {
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let waiting: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE pid=$1 AND wait_event_type='Lock')").bind(pid).fetch_one(&f.pool).await.expect("complete fixture database operation");
                if waiting { break; }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }).await.expect("promoter reached the locked definition");
        let before = tokio::time::timeout(Duration::from_secs(1), read(&f.service, scope, id))
            .await
            .expect("reader must not acquire promotion row locks");
        let before = retained(before);
        assert!(matches!(before.state(), IntentCorrelationState::Pending));
        assert_eq!(
            before
                .definition()
                .expect("registered definition")
                .version(),
            1
        );
        definition.commit().await.expect("commit definition change");
    };
    let (promoted, ()) = tokio::join!(promoting, observe);
    let promoted = promoted.expect("promote retained intent");
    promoter.close().await;
    assert_eq!(promoted.total_promoted, 1);
    let found = retained(read(&f.service, scope, id).await);
    assert_eq!(
        found.definition().expect("registered definition").version(),
        2
    );
    let job_id = match found.state() {
        IntentCorrelationState::Promoted(job) => {
            assert_eq!(job.run_number(), 1);
            job.id()
        }
        other => panic!("expected promoted job, got {other:?}"),
    };
    // A privileged malformed cross-tenant link must never reveal the other job.
    let other = support::enqueue_test_job(&f.pool, TYPE, Some(Uuid::now_v7()), &payload).await;
    sqlx::query("UPDATE job_enqueue_intents SET promoted_job_id=$1 WHERE id=$2")
        .bind(other)
        .bind(id)
        .execute(&f.pool)
        .await
        .expect("complete fixture database operation");
    let invalid = run_atomic(&f.service, async |mut tx| {
        tx.reconcile_job_enqueue_intent(scope, id).await
    })
    .await;
    assert!(matches!(invalid, Err(PgAtomicError::Rejected(_))));
    sqlx::query("UPDATE job_enqueue_intents SET promoted_job_id=$1 WHERE id=$2")
        .bind(job_id)
        .bind(id)
        .execute(&f.pool)
        .await
        .expect("complete fixture database operation");

    let mut retention = f
        .pool
        .begin()
        .await
        .expect("begin concurrent fixture transaction");
    runledger_postgres::jobs::delete_promoted_job_enqueue_intents_for_jobs_tx(
        &mut retention,
        &[job_id],
    )
    .await
    .expect("complete fixture database operation");
    sqlx::query("DELETE FROM job_queue WHERE id=$1")
        .bind(job_id)
        .execute(&mut *retention)
        .await
        .expect("complete fixture database operation");
    let before_commit = tokio::time::timeout(Duration::from_secs(3), read(&f.service, scope, id))
        .await
        .expect("snapshot read must not wait for retention row locks");
    assert!(matches!(
        retained(before_commit).state(),
        IntentCorrelationState::Promoted(_)
    ));
    retention.commit().await.expect("commit retention");
    assert!(matches!(
        read(&f.service, scope, id).await,
        IntentReconciliation::NotFound
    ));
    f.finish().await;
}

#[tokio::test]
async fn metadata_only_grants_cover_missing_definitions_and_conflicts() {
    use runledger_postgres::jobs::{JobEnqueue, enqueue_job, record_job_enqueue_intent};
    let f = Fixture::new("reconcile_metadata").await;
    let original = serde_json::json!({"private": "original"});
    let changed = serde_json::json!({"private": "changed"});
    let missing = record_job_enqueue_intent(
        &f.pool,
        &JobEnqueueIntent::new(JobType::new("test.intent.missing"), &original, "missing"),
    )
    .await
    .expect("complete fixture database operation");
    let conflict = record_job_enqueue_intent(
        &f.pool,
        &JobEnqueueIntent::new(JobType::new(TYPE), &original, "conflict"),
    )
    .await
    .expect("complete fixture database operation");
    enqueue_job(
        &f.pool,
        &JobEnqueue {
            job_type: JobType::new(TYPE),
            organization_id: None,
            payload: &changed,
            priority: None,
            max_attempts: None,
            timeout_seconds: None,
            next_run_at: None,
            idempotency_key: Some("conflict"),
            stage: None,
        },
    )
    .await
    .expect("complete fixture database operation");
    let report = promote_job_enqueue_intents_for_types(&f.pool, &[JobType::new(TYPE)], 10)
        .await
        .expect("complete fixture database operation");
    assert_eq!(report.total_promoted, 0);
    sqlx::query("UPDATE job_definitions SET is_enabled=false WHERE job_type=$1")
        .bind(TYPE)
        .execute(&f.pool)
        .await
        .expect("complete fixture database operation");
    f.restrict_to_reconciliation().await;
    let missing = retained(read(&f.service, JobScope::Global, missing.intent_id).await);
    assert!(missing.definition().is_none());
    assert!(matches!(missing.state(), IntentCorrelationState::Pending));
    let conflicted = retained(read(&f.service, JobScope::Global, conflict.intent_id).await);
    assert!(
        !conflicted
            .definition()
            .expect("registered definition")
            .is_enabled()
    );
    assert!(matches!(
        conflicted.state(),
        IntentCorrelationState::Conflicted
    ));
    for query in [
        "SELECT enqueue_request FROM job_enqueue_intents",
        "SELECT payload FROM job_queue",
        "SELECT last_error_message FROM job_enqueue_intents",
        "INSERT INTO job_enqueue_intents DEFAULT VALUES",
    ] {
        let error = sqlx::query(AssertSqlSafe(query))
            .execute(f.service.pool())
            .await
            .expect_err("metadata role must be denied this operation");
        assert_eq!(
            error
                .as_database_error()
                .expect("PostgreSQL permission error")
                .code()
                .as_deref(),
            Some("42501")
        );
    }
    f.finish().await;
}
