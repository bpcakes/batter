use super::super::{ControlPlaneStore, LoadPolicy, RegisterError, register_in};
use super::support::*;
use batter_sqlx::{PgProfiledPool, PgSessionProfile};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use std::time::Duration;

fn profile() -> PgSessionProfile {
    PgSessionProfile::new(
        "serving",
        "serving",
        vec!["public".into()],
        Duration::ZERO,
        Duration::ZERO,
    )
    .unwrap()
}

fn options() -> PgConnectOptions {
    PgConnectOptions::new_without_pgpass()
        .host("127.0.0.1")
        .username("serving")
        .database("serving")
}

fn registered_pool(target: &mut batter_core::lifecycle::Supervisor) -> PgProfiledPool {
    batter_sqlx::pool_in(
        target.reserve_cleanup("pool").unwrap(),
        PgPoolOptions::new(),
        options(),
        profile(),
    )
    .unwrap()
}

#[tokio::test]
async fn foreign_unregistered_and_extracted_pool_owners_are_refused_before_registration() {
    let mut owner = supervisor();
    let pool = registered_pool(&mut owner);
    let unregistered = batter_sqlx::low_level::connect_profiled_pool_lazy(
        options(),
        profile(),
        PgPoolOptions::new(),
    )
    .unwrap();
    for pool in [&pool, &unregistered] {
        let mut target = supervisor();
        let result = register_in(
            &mut target,
            ControlPlaneStore::new(pool, store_timing()),
            resolved(&[]).0,
            LoadPolicy::start_on_defaults(timing()),
            presence(),
        );
        assert!(matches!(result, Err(RegisterError::PoolOwnership)));
        assert!(target.take_cleanup().is_empty());
    }
    let cleanup = owner.take_cleanup();
    let result = register_in(
        &mut owner,
        ControlPlaneStore::new(&pool, store_timing()),
        resolved(&[]).0,
        LoadPolicy::start_on_defaults(timing()),
        presence(),
    );
    assert!(matches!(result, Err(RegisterError::PoolOwnership)));
    assert!(owner.take_cleanup().is_empty());
    assert!(
        cleanup
            .close(batter_core::cleanup::CleanupBudget::new(SECOND, SECOND, SECOND).unwrap())
            .await
            .is_success()
    );
    unregistered.pool().close().await;
}

#[tokio::test]
async fn moving_the_owner_preserves_registration_identity() {
    let mut owner = supervisor();
    let pool = registered_pool(&mut owner);
    let mut moved = Box::new(owner);
    let flags = register_in(
        moved.as_mut(),
        ControlPlaneStore::new(&pool, store_timing()),
        resolved(&[]).0,
        LoadPolicy::start_on_defaults(timing()),
        presence(),
    );
    assert!(flags.is_ok());
    // Pool closure and stop marker are both owned by this exact stack.
    assert_eq!(moved.take_cleanup().len(), 2);
    pool.pool().close().await;
}
