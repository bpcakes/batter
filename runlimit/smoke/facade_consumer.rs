use std::{cell::Cell, convert::Infallible, error::Error, time::Duration};

use batter::operation::OperationContext;
use batter::runlimit::{Checks, Quota, RunResult};
use runlimit_core::{Check, FixedWindowPolicy, KeyHasher, PolicyId, ScopeId};
use runlimit_memory::{MemoryStore, MemoryStoreConfig};

fn native_policy(value: batter::runlimit::native::FixedWindowPolicy) -> FixedWindowPolicy {
    value
}

fn native_postgres(
    value: batter::runlimit::postgres::PostgresLimiter,
) -> runlimit_postgres::PostgresLimiter {
    value
}

fn foundation(value: OperationContext) -> batter_core::operation::OperationContext {
    value
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    let _ = (native_policy, native_postgres, foundation);
    assert!(runlimit_axum::Admissions::default().is_empty());
    let _ = std::mem::size_of::<runlimit_http::draft_11::QuotaState>();
    let policy = FixedWindowPolicy::new(
        PolicyId::new("source.consume")?,
        ScopeId::new("owner")?,
        1,
        Duration::from_secs(3600),
    )?;
    let hasher = KeyHasher::new([7; 32])?;
    let checks = [Check::new(hasher.hash_for(&policy, "owner-a"))];
    // The native store must satisfy the facade and direct adapter's exact trait identity.
    let quota: batter_runlimit::Quota<MemoryStore> =
        Quota::new(MemoryStore::new(MemoryStoreConfig::new(100)?));
    let context = OperationContext::new(Duration::from_secs(10))?;
    let invoked = Cell::new(0);
    let first = quota
        .run(&context, Checks::new(&checks)?, |_| {
            invoked.set(invoked.get() + 1);
            async { Ok::<_, Infallible>(42) }
        })
        .await;
    assert!(matches!(first, RunResult::Admitted { work: Ok(42), .. }));
    let denied = quota
        .run(&context, Checks::new(&checks)?, |_| {
            invoked.set(invoked.get() + 1);
            async { Ok::<_, Infallible>(0) }
        })
        .await;
    assert!(matches!(denied, RunResult::Rejected { index: 0, .. }));
    assert_eq!(invoked.get(), 1, "denied quota must not invoke work");
    println!("facade quota admission and denial passed");
    Ok(())
}
