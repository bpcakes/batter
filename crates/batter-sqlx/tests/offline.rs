use batter_core::{
    lifecycle::{ShutdownBudget, Supervisor},
    operation::{Interruption, OperationError},
};
use batter_sqlx::verification::{
    AuthorityPolicyBuilder, DiscoveryScope, ExactRoleManifest, Identifier, MigrationExpectation,
    QualifiedName, SchemaInspectionPolicy, SqlxLedgerMode, SqlxMigrationManifest, VerificationPlan,
    verify, verify_exact_role, verify_sqlx_migrations,
};
use batter_sqlx::{FailureClass, PgLease, SqlxFailure, pool_in, probe, register_pool_close};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use std::{error::Error, time::Duration};

#[test]
fn diagnostics_hide_contents_and_keep_the_native_cause() {
    let error = SqlxFailure::from(sqlx::Error::Protocol("secret-protocol-marker".into()));
    assert_eq!(error.class(), FailureClass::Transport);
    assert!(!format!("{error:?} {error}").contains("secret-protocol-marker"));
    let source = error
        .source()
        .unwrap()
        .downcast_ref::<sqlx::Error>()
        .unwrap();
    assert!(std::ptr::eq(source, error.native()));
    assert!(
        matches!(error.into_native(), sqlx::Error::Protocol(text) if text == "secret-protocol-marker")
    );
    // Text that sounds transient never overrides the native variant.
    let error = SqlxFailure::from(sqlx::Error::InvalidArgument(
        "retry transient timeout".into(),
    ));
    assert_eq!(error.class(), FailureClass::Other);
    assert_eq!(
        SqlxFailure::from(sqlx::Error::PoolTimedOut).class(),
        FailureClass::PoolUnavailable
    );
}

fn lazy_pool() -> sqlx::PgPool {
    PgPoolOptions::new()
        .connect_lazy("postgres://unused:secret-url-marker@127.0.0.1:1/unused")
        .unwrap()
}

#[tokio::test]
async fn inert_and_interrupted_calls_do_not_acquire() {
    let pool = lazy_pool();
    let owner = batter_core::operation::OperationOwner::new(Duration::from_secs(1)).unwrap();
    let context = owner.context().clone();
    drop(PgLease::acquire(&pool, &context));
    drop(probe(&pool, &context));
    owner.cancel();
    assert!(matches!(
        PgLease::acquire(&pool, &context).await,
        Err(OperationError::Interrupted(Interruption::Cancelled))
    ));
    assert!(matches!(
        probe(&pool, &context).await,
        Err(OperationError::Interrupted(Interruption::Cancelled))
    ));
    assert_eq!(pool.size(), 0);
    let expired = batter_core::operation::OperationOwner::at(
        batter_core::operation::RootDeadline::at(tokio::time::Instant::now()),
    )
    .into_context();
    assert!(matches!(
        probe(&pool, &expired).await,
        Err(OperationError::Interrupted(Interruption::DeadlineExceeded))
    ));
    pool.close().await;
}

#[tokio::test]
async fn interrupted_valid_plans_do_not_acquire() {
    let pool = lazy_pool();
    let ledger = SqlxMigrationManifest::new(
        QualifiedName::new("public", "_sqlx_migrations").unwrap(),
        SqlxLedgerMode::Exact,
        [MigrationExpectation::new(1, [1])],
    )
    .unwrap();
    let schema = SchemaInspectionPolicy::canonical([Identifier::new("service").unwrap()]).unwrap();
    let role = ExactRoleManifest::new(
        Identifier::new("service").unwrap(),
        DiscoveryScope::Declared,
    )
    .unwrap()
    .compile()
    .unwrap();
    let owner = batter_core::operation::OperationOwner::new(Duration::from_secs(1)).unwrap();
    owner.cancel();
    let context = owner.context().clone();
    assert!(matches!(
        verify_sqlx_migrations(&pool, &context, &ledger).await,
        Err(OperationError::Interrupted(Interruption::Cancelled))
    ));
    assert!(matches!(
        verify_exact_role(&pool, &context, &role).await,
        Err(OperationError::Interrupted(Interruption::Cancelled))
    ));
    assert!(matches!(
        verify(
            &pool,
            &context,
            VerificationPlan::schema_inspection(&schema)
        )
        .await,
        Err(OperationError::Interrupted(Interruption::Cancelled))
    ));
    assert_eq!(pool.size(), 0);
    pool.close().await;
}

#[test]
fn invalid_policy_drafts_fail_before_an_executable_value_exists() {
    let mut draft = AuthorityPolicyBuilder::default();
    draft
        .required_privileges
        .push(batter_sqlx::verification::RequiredPrivilege {
            object: batter_sqlx::verification::PublicObject::Database,
            privilege: batter_sqlx::verification::ObjectPrivilege::Execute,
        });
    assert_eq!(
        draft.build(),
        Err(batter_sqlx::verification::PolicyError::InvalidObjectPrivilege)
    );
}

#[tokio::test]
async fn closed_pool_failure_is_native_and_distinct_from_interruption() {
    let pool = lazy_pool();
    pool.close().await;
    let context = batter_core::operation::OperationOwner::new(Duration::from_secs(1))
        .unwrap()
        .into_context();
    let Err(OperationError::Failed(error)) = probe(&pool, &context).await else {
        panic!("expected native failure")
    };
    assert!(matches!(error.native(), sqlx::Error::PoolClosed));
    assert_eq!(error.class(), FailureClass::PoolUnavailable);
}

#[tokio::test]
async fn rejected_registration_retains_caller_pool_ownership() {
    let pool = lazy_pool();
    let second = Duration::from_secs(1);
    let cleanup = batter_core::cleanup::CleanupBudget::new(second, second, second).unwrap();
    let mut supervisor =
        Supervisor::new(ShutdownBudget::new(second, second, second, cleanup).unwrap());
    assert!(register_pool_close(&mut supervisor, "", &pool).is_err());
    assert!(!pool.is_closed());
    register_pool_close(&mut supervisor, "pool", &pool).unwrap();
    assert!(register_pool_close(&mut supervisor, "pool", &pool).is_err());
    let report = supervisor.take_cleanup().close(cleanup).await;
    assert!(report.is_success());
    assert_eq!(report.records.len(), 1);
    assert!(pool.is_closed());
}

#[tokio::test]
async fn slot_owned_pool_registers_close_before_return() {
    let second = Duration::from_secs(1);
    let budget = batter_core::cleanup::CleanupBudget::new(second, second, second).unwrap();
    let mut cleanup = batter_core::cleanup::CleanupStack::new();
    let pool = pool_in(
        cleanup.reserve("pool").unwrap(),
        PgPoolOptions::new(),
        PgConnectOptions::new()
            .host("127.0.0.1")
            .port(1)
            .username("unused")
            .database("unused"),
    );
    let retained = pool.clone();
    let report = cleanup.close(budget).await;
    assert!(report.is_success());
    assert_eq!(report.records.len(), 1);
    assert_eq!(report.records[0].name, "pool");
    assert!(retained.is_closed());
    assert!(matches!(
        retained.acquire().await,
        Err(sqlx::Error::PoolClosed)
    ));
}

#[test]
fn reservation_rejection_precedes_pool_construction() {
    let mut cleanup = batter_core::cleanup::CleanupStack::new();
    assert!(cleanup.reserve("").is_err());
    let first = cleanup.reserve("pool").unwrap();
    first.register(|| async { Ok(()) });
    assert!(cleanup.reserve("pool").is_err());
}

#[test]
fn legacy_registration_signature_remains_exact() {
    let _: fn(
        &mut Supervisor,
        &'static str,
        &sqlx::PgPool,
    ) -> Result<(), batter_core::RegistrationError> = register_pool_close;
}
