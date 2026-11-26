use std::{
    fmt::Debug,
    marker::PhantomData,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use runlimit_core::{
    BatchDecision, BatchDecisionView, Check, Decision, Denial, FixedWindowPolicy, GcraPolicy,
    PolicyId, QuotaMode, ScopeId, SubjectKey,
};

use super::{
    BoundedShards, LockedShard, PreflightedSingle, QuotaAlgorithm, QuotaEvaluation, ShardEffect,
    SinglePreflight,
    batch::{BatchEffects, BatchPreflight, LockedBatch, PreflightedBatch},
    execute_batch, execute_check,
};
use crate::{Clock, MemoryStoreConfig, gcra::GcraAlgorithm, store::FixedWindowAlgorithm};

#[derive(Clone, Default)]
struct ManualClock {
    millis: Arc<AtomicU64>,
}

impl ManualClock {
    fn advance(&self, duration: Duration) {
        let millis = u64::try_from(duration.as_millis()).unwrap();
        self.millis.fetch_add(millis, Ordering::Relaxed);
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Duration {
        Duration::from_millis(self.millis.load(Ordering::Relaxed))
    }
}

fn subject(byte: u8) -> SubjectKey {
    SubjectKey::from_digest([byte; 32])
}

#[derive(Debug, Eq, PartialEq)]
struct StoreSnapshot<E> {
    shards: Vec<ShardSnapshot<E>>,
}

#[derive(Debug, Eq, PartialEq)]
struct ShardSnapshot<E> {
    used: usize,
    expirations: usize,
    entries: Vec<(runlimit_core::CounterKey, E, u128)>,
}

fn capture_snapshot<E: Copy>(shards: &BoundedShards<E>) -> StoreSnapshot<E> {
    let mut snapshots = Vec::with_capacity(shards.len());
    for index in 0..shards.len() {
        let shard = shards.shard(index).lock().unwrap();
        snapshots.push(ShardSnapshot {
            used: shard.used(),
            expirations: shard.expiration_count(),
            entries: shard.snapshot(),
        });
    }
    StoreSnapshot { shards: snapshots }
}

fn assert_batch_preflight_denied<A>(
    shards: &BoundedShards<A::Entry>,
    config: &MemoryStoreConfig,
    clock: &ManualClock,
    checks: &[Check<'_, A::Policy>],
    baseline: &StoreSnapshot<A::Entry>,
    expected_index: usize,
) where
    A: QuotaAlgorithm,
    A::Entry: Copy + Debug + Eq,
    A::EvalError: Debug,
{
    let preflight = LockedBatch::<A>::lock(shards, checks, config, clock)
        .unwrap()
        .prepare()
        .unwrap()
        .advance_and_prune(config.max_expired_removals_per_check(), true)
        .preflight()
        .unwrap();
    assert!(matches!(
        preflight,
        BatchPreflight::Denied { decision, .. }
            if matches!(
                decision.view(),
                BatchDecisionView::Denied {
                    index,
                    denial: Denial::QuotaExceeded(_),
                    ..
                } if index == expected_index
            )
    ));
    assert_eq!(capture_snapshot(shards), *baseline);
}

fn assert_single_preflight_denied<A>(
    shards: &BoundedShards<A::Entry>,
    config: &MemoryStoreConfig,
    clock: &ManualClock,
    check: &Check<'_, A::Policy>,
    baseline: &StoreSnapshot<A::Entry>,
) where
    A: QuotaAlgorithm,
    A::Entry: Copy + Debug + Eq,
    A::EvalError: Debug,
{
    let preflight = LockedShard::<A>::lock(shards, check, clock)
        .unwrap()
        .evaluate()
        .unwrap()
        .advance_and_prune(config.max_expired_removals_per_check(), true)
        .preflight();
    assert!(matches!(
        preflight,
        SinglePreflight::Denied { decision, .. } if !decision.permits_request()
    ));
    assert_eq!(capture_snapshot(shards), *baseline);
}

fn assert_denied_preflight_preserves_every_shard_and_entry<A>(
    enforced_multi: &A::Policy,
    enforced_single: &A::Policy,
    shadow_multi: &A::Policy,
    shadow_single: &A::Policy,
) where
    A: QuotaAlgorithm,
    A::Entry: Copy + Debug + Eq,
    A::EvalError: Debug,
{
    let clock = ManualClock::default();
    let config = MemoryStoreConfig::new(2)
        .unwrap()
        .with_shard_count(1)
        .unwrap();
    let shards = BoundedShards::<A::Entry>::new(&config);

    // Seed two active entries: one with remaining quota and one exhausted.
    let active_with_room = Check::new(subject(1).bind(enforced_multi));
    let exhausted = Check::new(subject(2).bind(enforced_single));
    let new_key = Check::new(subject(3).bind(enforced_multi));
    assert!(
        execute_check::<A, _>(&shards, &config, &clock, None, &active_with_room)
            .unwrap()
            .permits_request()
    );
    assert!(
        execute_check::<A, _>(&shards, &config, &clock, None, &exhausted)
            .unwrap()
            .permits_request()
    );

    let baseline = capture_snapshot(&shards);
    assert_eq!(baseline.shards[0].used, 2);
    assert_eq!(baseline.shards[0].entries.len(), 2);

    // 1. Enforced quota denial after an earlier allowed member yields no
    //    PreflightedBatch and leaves every stored entry unchanged.
    let quota_denied_checks = [active_with_room, exhausted];
    assert_batch_preflight_denied::<A>(
        &shards,
        &config,
        &clock,
        &quota_denied_checks,
        &baseline,
        1,
    );

    // 2. Shadow quota denial after an earlier allowed member leaves every
    //    stored entry unchanged.
    let shadow_checks = [
        Check::new(subject(1).bind(shadow_multi)),
        Check::new(subject(2).bind(shadow_single)),
    ];
    let shadow_decision =
        execute_batch::<A, _>(&shards, &config, &clock, None, &shadow_checks).unwrap();
    assert!(matches!(
        shadow_decision.view(),
        BatchDecisionView::ShadowDenied { index: 1, .. }
    ));
    assert_eq!(capture_snapshot(&shards), baseline);

    // 3. Storage-capacity denial after an earlier allowed member leaves every
    //    stored entry unchanged.
    let capacity_decision =
        execute_batch::<A, _>(&shards, &config, &clock, None, &[active_with_room, new_key])
            .unwrap();
    assert!(matches!(
        capacity_decision.view(),
        BatchDecisionView::Denied {
            index: 1,
            denial: Denial::StorageCapacity { .. },
            ..
        }
    ));
    assert_eq!(capture_snapshot(&shards), baseline);

    // 4. Storage-capacity denial at an earlier index supersedes a later quota
    //    denial and leaves every stored entry unchanged.
    let precedence_decision =
        execute_batch::<A, _>(&shards, &config, &clock, None, &[new_key, exhausted]).unwrap();
    assert!(matches!(
        precedence_decision.view(),
        BatchDecisionView::Denied {
            index: 0,
            denial: Denial::StorageCapacity { .. },
            ..
        }
    ));
    assert_eq!(capture_snapshot(&shards), baseline);

    // 5. Single-check denied preflight yields no PreflightedSingle and leaves
    //    every stored entry unchanged.
    assert_single_preflight_denied::<A>(&shards, &config, &clock, &exhausted, &baseline);

    // 6. Advancing time within the window still leaves denied entries untouched.
    clock.advance(Duration::from_millis(10));
    let _ = execute_batch::<A, _>(&shards, &config, &clock, None, &quota_denied_checks).unwrap();
    assert_eq!(capture_snapshot(&shards), baseline);
}

#[test]
fn denied_preflight_leaves_used_and_all_stored_values_unchanged_across_both_stores() {
    let fw_multi = FixedWindowPolicy::new(
        PolicyId::new("shared.multi").unwrap(),
        ScopeId::new("client").unwrap(),
        5,
        Duration::from_secs(60),
    )
    .unwrap();
    let fw_single = FixedWindowPolicy::new(
        PolicyId::new("shared.single").unwrap(),
        ScopeId::new("client").unwrap(),
        1,
        Duration::from_secs(60),
    )
    .unwrap();
    let fw_shadow_multi = fw_multi.clone().with_quota_mode(QuotaMode::Shadow);
    let fw_shadow_single = fw_single.clone().with_quota_mode(QuotaMode::Shadow);
    assert_denied_preflight_preserves_every_shard_and_entry::<FixedWindowAlgorithm>(
        &fw_multi,
        &fw_single,
        &fw_shadow_multi,
        &fw_shadow_single,
    );

    let gcra_multi = GcraPolicy::new(
        PolicyId::new("shared.multi").unwrap(),
        ScopeId::new("client").unwrap(),
        5,
        Duration::from_secs(60),
        5,
    )
    .unwrap();
    let gcra_single = GcraPolicy::new(
        PolicyId::new("shared.single").unwrap(),
        ScopeId::new("client").unwrap(),
        1,
        Duration::from_secs(60),
        1,
    )
    .unwrap();
    let gcra_shadow_multi = gcra_multi.clone().with_quota_mode(QuotaMode::Shadow);
    let gcra_shadow_single = gcra_single.clone().with_quota_mode(QuotaMode::Shadow);
    assert_denied_preflight_preserves_every_shard_and_entry::<GcraAlgorithm>(
        &gcra_multi,
        &gcra_single,
        &gcra_shadow_multi,
        &gcra_shadow_single,
    );
}

fn assert_commit_single<'a, A: QuotaAlgorithm>(
    _: fn(PreflightedSingle<'a, A>) -> (Decision, ShardEffect),
) {
}

fn assert_commit_batch<'a, A: QuotaAlgorithm>(
    _: fn(PreflightedBatch<'a, A>) -> (BatchDecision, BatchEffects),
) {
}

#[test]
fn commit_consumes_the_preflighted_owner_and_tokens_are_single_use() {
    // Compile-time !Copy and !Clone assertions: adding Copy or Clone to any
    // token or preflighted owner causes an E0034 method-resolution ambiguity.
    trait NotCopy {
        fn assert_not_copy() {}
    }
    impl<T> NotCopy for PhantomData<T> {}
    #[expect(dead_code)]
    trait CopyConflict {
        fn assert_not_copy() {}
    }
    impl<T: Copy> CopyConflict for PhantomData<T> {}

    trait NotClone {
        fn assert_not_clone() {}
    }
    impl<T> NotClone for PhantomData<T> {}
    #[expect(dead_code)]
    trait CloneConflict {
        fn assert_not_clone() {}
    }
    impl<T: Clone> CloneConflict for PhantomData<T> {}

    PhantomData::<<FixedWindowAlgorithm as QuotaAlgorithm>::Token>::assert_not_copy();
    PhantomData::<<FixedWindowAlgorithm as QuotaAlgorithm>::Token>::assert_not_clone();
    PhantomData::<<GcraAlgorithm as QuotaAlgorithm>::Token>::assert_not_copy();
    PhantomData::<<GcraAlgorithm as QuotaAlgorithm>::Token>::assert_not_clone();
    PhantomData::<PreflightedSingle<'_, FixedWindowAlgorithm>>::assert_not_copy();
    PhantomData::<PreflightedSingle<'_, FixedWindowAlgorithm>>::assert_not_clone();
    PhantomData::<PreflightedBatch<'_, FixedWindowAlgorithm>>::assert_not_copy();
    PhantomData::<PreflightedBatch<'_, FixedWindowAlgorithm>>::assert_not_clone();
    PhantomData::<PreflightedSingle<'_, GcraAlgorithm>>::assert_not_copy();
    PhantomData::<PreflightedSingle<'_, GcraAlgorithm>>::assert_not_clone();
    PhantomData::<PreflightedBatch<'_, GcraAlgorithm>>::assert_not_copy();
    PhantomData::<PreflightedBatch<'_, GcraAlgorithm>>::assert_not_clone();

    // Both terminal write paths require a whole preflighted owner by value.
    assert_commit_single::<FixedWindowAlgorithm>(PreflightedSingle::commit);
    assert_commit_single::<GcraAlgorithm>(PreflightedSingle::commit);
    assert_commit_batch::<FixedWindowAlgorithm>(PreflightedBatch::commit);
    assert_commit_batch::<GcraAlgorithm>(PreflightedBatch::commit);

    // Also verify a denied evaluation produces no token for either algorithm.
    let config = MemoryStoreConfig::new(1).unwrap();
    let fw_shards = BoundedShards::<<FixedWindowAlgorithm as QuotaAlgorithm>::Entry>::new(&config);
    let fw_policy = FixedWindowPolicy::new(
        PolicyId::new("token.fw").unwrap(),
        ScopeId::new("client").unwrap(),
        1,
        Duration::from_secs(60),
    )
    .unwrap();
    let fw_check = Check::new(subject(1).bind(&fw_policy));
    execute_check::<FixedWindowAlgorithm, _>(
        &fw_shards,
        &config,
        &ManualClock::default(),
        None,
        &fw_check,
    )
    .unwrap();
    let fw_guard = fw_shards.shard(0).lock().unwrap();
    assert!(matches!(
        FixedWindowAlgorithm::evaluate(&fw_guard, &fw_check, 0),
        Ok(QuotaEvaluation::Denied(_))
    ));
}
