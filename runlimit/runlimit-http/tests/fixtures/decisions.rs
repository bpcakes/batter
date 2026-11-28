//! Validated policy and decision fixtures shared by the draft test targets.

use std::time::Duration;

use runlimit_core::{
    Allowance, Capacity, FixedWindowPolicy, GcraPolicy, PolicyId, QuotaDenial, ScopeId,
};

pub fn fixed_policy(limit: u64, window: Duration) -> FixedWindowPolicy {
    FixedWindowPolicy::new(
        PolicyId::new("api.search").unwrap(),
        ScopeId::new("client").unwrap(),
        limit,
        window,
    )
    .unwrap()
}

pub fn gcra_policy(limit: u64, window: Duration) -> GcraPolicy {
    GcraPolicy::new(
        PolicyId::new("api.upload").unwrap(),
        ScopeId::new("account").unwrap(),
        limit,
        window,
        4,
    )
    .unwrap()
}

pub fn allowance(capacity: u64, available: u64, replenishes_after: Duration) -> Allowance {
    Allowance::new(
        Capacity::new(capacity).unwrap(),
        available,
        replenishes_after,
    )
    .unwrap()
}

pub fn quota(capacity: u64, retry_after: Duration) -> QuotaDenial {
    QuotaDenial::new(Capacity::new(capacity).unwrap(), retry_after)
}
