//! Compile-time and runtime coverage for the shared limiter abstraction.

use std::{fmt::Debug, time::Duration};

use runlimit_core::{
    Allowance, BatchDecision, BatchDecisionView, BatchError, Check, Decision, FixedWindowPolicy,
    Limiter, PolicyId, ScopeId, SubjectKey,
};
use runlimit_memory::{MemoryBatchError, MemoryStore, MemoryStoreConfig};
use runlimit_postgres::{BatchCheckError, PostgresLimiter};
use sqlx::postgres::PgPoolOptions;

async fn check_batch<L>(
    limiter: &L,
    checks: &[Check<'_>],
) -> Result<BatchDecision, L::CheckAllError>
where
    L: Limiter<Policy = FixedWindowPolicy>,
{
    limiter.check_all(checks).await
}

async fn check_one<L>(limiter: &L, check: &Check<'_>) -> Result<Decision, L::CheckError>
where
    L: Limiter<Policy = FixedWindowPolicy>,
{
    limiter.check(check).await
}

fn assert_send<T: Send>(_: T) {}

fn policy(name: &str, limit: u64) -> FixedWindowPolicy {
    FixedWindowPolicy::new(
        PolicyId::new(name).expect("test policy ID is valid"),
        ScopeId::new("subject").expect("test scope ID is valid"),
        limit,
        Duration::from_mins(1),
    )
    .expect("test policy is valid")
}

fn key(byte: u8) -> SubjectKey {
    SubjectKey::from_digest([byte; 32])
}

fn expect_allowed<E: Debug>(result: Result<BatchDecision, E>) -> Vec<Allowance> {
    match result.expect("generic limiter call succeeds").view() {
        BatchDecisionView::Allowed { allowances } => allowances.to_vec(),
        denied => panic!("generic limiter call should be allowed: {denied:?}"),
    }
}

#[tokio::test]
async fn one_generic_function_swaps_between_backends() {
    let memory = MemoryStore::new(
        MemoryStoreConfig::new(4).expect("test memory-store configuration is valid"),
    );
    let postgres = PostgresLimiter::new(
        PgPoolOptions::new()
            .connect_lazy("postgresql://runlimit:runlimit@127.0.0.1:1/runlimit")
            .expect("test database URL is valid"),
    );
    let single_policy = policy("single", 2);
    let single_check = Check::new(key(9).bind(&single_policy));
    let duplicate_checks = [single_check, single_check];

    assert!(
        check_one(&memory, &single_check)
            .await
            .unwrap()
            .permits_request()
    );
    assert_eq!(
        check_batch(&memory, &[]).await,
        Err(MemoryBatchError::InvalidBatch(BatchError::EmptyBatch))
    );
    assert!(
        matches!(
            check_batch(&postgres, &[]).await,
            Err(BatchCheckError::InvalidBatch(BatchError::EmptyBatch))
        ),
        "an empty PostgreSQL batch is rejected before a connection is needed"
    );
    assert!(matches!(
        check_batch(&memory, &duplicate_checks).await,
        Err(MemoryBatchError::InvalidBatch(BatchError::DuplicateKey {
            first_index: 0,
            duplicate_index: 1,
        }))
    ));
    assert!(matches!(
        check_batch(&postgres, &duplicate_checks).await,
        Err(BatchCheckError::InvalidBatch(BatchError::DuplicateKey {
            first_index: 0,
            duplicate_index: 1,
        }))
    ));

    assert_send(Limiter::check(&memory, &single_check));
    assert_send(Limiter::check(&postgres, &single_check));
    assert_send(Limiter::check_all(&memory, &[]));
    assert_send(Limiter::check_all(&postgres, &[]));
}

#[tokio::test]
async fn generic_batch_preserves_caller_order() {
    let first_policy = policy("first", 11);
    let second_policy = policy("second", 7);
    let checks = [
        Check::new(key(1).bind(&first_policy))
            .with_cost(3)
            .expect("test check is valid"),
        Check::new(key(2).bind(&second_policy))
            .with_cost(2)
            .expect("test check is valid"),
    ];
    let memory = MemoryStore::new(
        MemoryStoreConfig::new(4).expect("test memory-store configuration is valid"),
    );

    let allowances = expect_allowed(check_batch(&memory, &checks).await);

    assert_eq!(allowances.len(), 2);
    assert_eq!(
        (allowances[0].capacity().get(), allowances[0].available()),
        (11, 8)
    );
    assert_eq!(
        (allowances[1].capacity().get(), allowances[1].available()),
        (7, 5)
    );
}
