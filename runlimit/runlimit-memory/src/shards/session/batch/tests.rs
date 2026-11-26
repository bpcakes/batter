use std::{cell::Cell, time::Duration};

use runlimit_core::{
    BatchDecisionView, Check, Denial, FixedWindowPolicy, PolicyId, ScopeId, SubjectKey,
};

use super::BoundedShards;
use crate::{
    Clock, MemoryStoreConfig,
    shards::{execute_batch, execute_check},
    store::FixedWindowAlgorithm,
};

thread_local! { static EVALUATIONS: Cell<usize> = const { Cell::new(0) }; }

struct FrozenClock;

impl Clock for FrozenClock {
    fn now(&self) -> Duration {
        Duration::ZERO
    }
}

pub(super) fn count_evaluation() {
    EVALUATIONS.update(|count| count + 1);
}

#[test]
fn fixed_window_stops_quota_lookups_on_both_quota_and_capacity_denials() {
    let clock = FrozenClock;
    let config = MemoryStoreConfig::new(3).unwrap();
    let shards = BoundedShards::new(config);
    let policy = FixedWindowPolicy::new(
        PolicyId::new("batch.denial").unwrap(),
        ScopeId::new("client").unwrap(),
        2,
        Duration::from_secs(60),
    )
    .unwrap();
    let [available, exhausted, occupied, new_key] =
        [1, 2, 3, 4].map(|n| Check::new(SubjectKey::from_digest([n; 32]).bind(&policy)));
    for check in [available, exhausted, exhausted, occupied] {
        assert!(
            execute_check::<FixedWindowAlgorithm, _>(&shards, &clock, None, &check)
                .unwrap()
                .permits_request()
        );
    }
    for (first, capacity_denied) in [(exhausted, false), (new_key, true)] {
        EVALUATIONS.set(0);
        let decision = execute_batch::<FixedWindowAlgorithm, _>(
            &shards,
            &clock,
            None,
            &[available, first, occupied],
        )
        .unwrap();
        let BatchDecisionView::Denied { index, denial, .. } = decision.view() else {
            panic!("expected a batch denial");
        };
        assert_eq!(index, 1);
        assert_eq!(
            matches!(denial, Denial::StorageCapacity { .. }),
            capacity_denied
        );
        assert_eq!(EVALUATIONS.get(), 2);
    }
}
