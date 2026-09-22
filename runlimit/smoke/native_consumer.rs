use std::{env, error::Error, time::Duration};

use runlimit_core::{
    AdmissionObservation, AdmissionOperation, BatchDecisionView, Check, CleanupObservation,
    CleanupOutcome, ConsumptionStatus, Denial, FixedWindowPolicy, KeyHasher, PolicyId, ScopeId,
};
use runlimit_memory::{
    GcraBatchError, GcraCheckError, MemoryBatchError, MemoryStore, MemoryStoreConfig,
    PoisonedShardError,
};

fn main() -> Result<(), Box<dyn Error>> {
    fn accepts_gcra<L: runlimit_core::Limiter<Policy = runlimit_core::GcraPolicy>>() {}
    check_attempt_api()?;
    accepts_gcra::<runlimit_postgres::PostgresGcraLimiter>();
    assert!(!runlimit_postgres::CREATE_RUNLIMIT_GCRA_SQL.is_empty());
    assert!(!runlimit_postgres::GCRA_MIGRATOR.migrations.is_empty());
    let gcra_error = GcraBatchError::from(MemoryBatchError::from(PoisonedShardError {
        shard_index: 0,
    }));
    assert!(matches!(
        gcra_error,
        GcraBatchError::Store(MemoryBatchError::PoisonedShard(PoisonedShardError {
            shard_index: 0
        }))
    ));
    for gcra_check_error in [
        GcraCheckError::from(PoisonedShardError { shard_index: 0 }),
        GcraCheckError::ArithmeticOverflow,
    ] {
        match gcra_check_error {
            GcraCheckError::PoisonedShard(error) => assert_eq!(error.shard_index, 0),
            GcraCheckError::ArithmeticOverflow => {}
        }
    }

    let cleanup = CleanupObservation::new(100, CleanupOutcome::Unknown, Duration::from_millis(5));
    assert_eq!(cleanup.outcome(), CleanupOutcome::Unknown);

    let client_policy = FixedWindowPolicy::new(
        PolicyId::new("auth.login")?,
        ScopeId::new("client")?,
        40,
        Duration::from_mins(1),
    )?;
    let identity_policy = FixedWindowPolicy::new(
        PolicyId::new("auth.login")?,
        ScopeId::new("identity")?,
        8,
        Duration::from_mins(1),
    )?;

    let secret = env::var("RUNLIMIT_KEY_SECRET")?;
    let key_hasher = KeyHasher::new(secret.as_bytes())?;

    // Address extraction and subject normalization remain application-owned.
    let client = key_hasher.hash_for(&client_policy, b"client-network:192.0.2.4");
    let identity = key_hasher.hash_for(&identity_policy, b"user@example.test");

    let config = MemoryStoreConfig::new(50_000)?.with_shard_count(64)?;
    let limiter = MemoryStore::new(config);
    let checks = [Check::new(client), Check::new(identity)];
    let failed_batch = AdmissionObservation::failed_batch(
        &checks[..1],
        ConsumptionStatus::NotConsumed,
        Duration::from_millis(5),
    );
    match failed_batch.operation() {
        AdmissionOperation::Batch {
            batch_size: 1,
            policy: Some(policy),
        } => assert_eq!(policy.id().as_str(), "auth.login"),
        other => panic!("a one-check batch names its policy: {other:?}"),
    }

    let decision = limiter.check_all(&checks)?;
    match decision.view() {
        BatchDecisionView::Allowed { allowances } => {
            assert_eq!(allowances.len(), checks.len());
            println!("request admitted");
        }
        BatchDecisionView::Denied {
            index,
            denial: Denial::QuotaExceeded(quota),
            ..
        } => {
            let seconds = quota.retry_after().seconds();
            println!("check {index} denied; retry after {seconds} seconds");
        }
        BatchDecisionView::Denied {
            index,
            denial: Denial::StorageCapacity { .. },
            ..
        } => println!("check {index} denied; backend storage is full"),
        BatchDecisionView::ShadowDenied { index, .. } => {
            println!("request admitted after check {index} was shadow denied");
        }
    }

    Ok(())
}

fn check_attempt_api() -> Result<(), Box<dyn Error>> {
    use runlimit_core::{
        Delay, QuotaPeriod,
        attempts::{
            AttemptAdmission, AttemptCompletion, AttemptCompletionResult, AttemptOutcome,
            AttemptPolicy, AttemptPolicyError,
        },
    };
    use runlimit_memory::attempts::MemoryAttemptLimiter;
    let period = QuotaPeriod::new(Duration::from_secs(1))?;
    let policy = AttemptPolicy::new(
        PolicyId::new("login")?,
        ScopeId::new("account")?,
        period,
        period,
        period,
        period,
    )?;
    let invalid = AttemptPolicy::new(
        policy.id().clone(),
        policy.scope().clone(),
        period,
        QuotaPeriod::new(Duration::from_millis(1))?,
        period,
        period,
    );
    assert_eq!(invalid, Err(AttemptPolicyError::MaximumBelowInitial));
    assert!(
        AttemptCompletion::new(AttemptOutcome::Success, 1, Delay::new(Duration::ZERO)).is_err()
    );
    let limiter = MemoryAttemptLimiter::new(std::num::NonZeroUsize::new(2).unwrap());
    let hasher = KeyHasher::new([7; 32])?;
    let AttemptAdmission::Admitted(receipt) =
        limiter.admit(hasher.hash_attempt_for(&policy, b"opaque-subject"))?
    else {
        panic!("empty store denied")
    };
    let AttemptCompletionResult::Applied(completion) =
        limiter.complete(receipt, AttemptOutcome::Success)?
    else {
        panic!("fresh receipt stale")
    };
    assert_eq!(completion.consecutive_failures(), 0);
    assert!(!runlimit_postgres::attempts::CREATE_RUNLIMIT_ATTEMPTS_SQL.is_empty());
    Ok(())
}
