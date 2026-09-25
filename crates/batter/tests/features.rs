//! Facade feature and type-identity checks. Isolated consumer graphs provide
//! the per-feature optionality proof; these tests pin the in-workspace module
//! paths and direct adapter identities when the feature set is enabled.

#[test]
fn foundation_paths_remain_available_with_any_facade_feature_set() {
    let _: Option<batter::operation::OperationContext> = None;
    let _: Option<batter_core::operation::OperationContext> = None;
}

#[cfg(feature = "at-rest")]
#[test]
fn at_rest_facade_rotates_a_complete_direct_package_envelope() -> Result<(), batter::at_rest::Error>
{
    use batter::at_rest::{Context, KeyId, Keyring, SecretKey};

    let id = KeyId::new("current")?;
    let keyring = Keyring::new(id.clone(), [(id, SecretKey::from_bytes([7; 32]))])?;
    let context = Context::for_row("example", "record", b"owner", b"item", "bytes-v1")?;
    let sealed = keyring.seal(&context, b"payload")?;
    let replacement: batter_at_rest::Envelope =
        keyring.rewrap_envelope(&context, sealed.envelope())?;

    assert_eq!(replacement, *sealed.envelope());
    Ok(())
}

#[cfg(feature = "axum")]
#[test]
fn axum_namespace_reexports_the_direct_adapter_types() {
    fn direct(_: batter_axum::RequestPolicy) {}
    let _: fn(batter::axum::RequestPolicy) = direct;
}

#[cfg(feature = "sqlx")]
#[test]
fn sqlx_namespace_reexports_the_direct_adapter_types() {
    fn direct(_: batter_sqlx::PgLease) {}
    let _: fn(batter::sqlx::PgLease) = direct;
}

#[cfg(feature = "sqlx-test-support")]
#[test]
fn sqlx_fixture_namespace_reexports_the_direct_fixture_types() {
    fn direct(_: batter_sqlx::test_support::ConnectionPlan) {}
    let _: fn(batter::sqlx::test_support::ConnectionPlan) = direct;
}

#[cfg(feature = "runledger")]
#[test]
fn runledger_namespace_reexports_the_direct_adapter_types() {
    fn direct(_: batter_runledger::NativeReport) {}
    let _: fn(batter::runledger::NativeReport) = direct;
}

#[cfg(feature = "runlimit")]
#[test]
fn runlimit_namespace_reexports_the_direct_adapter_types() {
    fn direct(_: batter_runlimit::EmptyChecks) {}
    let _: fn(batter::runlimit::EmptyChecks) = direct;
}

#[cfg(feature = "runlimit-axum")]
#[test]
fn runlimit_http_namespace_reexports_the_direct_adapter_types() {
    fn direct(_: batter_runlimit::http::DirectPeer) {}
    let _: fn(batter::runlimit::http::DirectPeer) = direct;
}

#[cfg(feature = "test-support")]
#[test]
fn generic_test_support_reexports_the_direct_types() {
    fn direct(_: batter_test_support::Script<(), ()>) {}
    let _: fn(batter::test_support::Script<(), ()>) = direct;
}
