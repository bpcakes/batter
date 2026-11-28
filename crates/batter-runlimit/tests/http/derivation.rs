//! The boundary derives each protected subject key itself: the stored
//! identity matches the native derivation the old selector returned, and the
//! selector's material may borrow from the principal.

use super::*;
use runlimit_core::Check;

/// A principal that owns its id, so the selector's material is a borrow.
#[derive(Clone, Debug)]
struct OwnedPrincipal(String);

async fn owner_from(input: AuthInput) -> Result<OwnedPrincipal, AuthError> {
    match input
        .headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
    {
        Some("Bearer secret-a") => Ok(OwnedPrincipal("owner-a".to_owned())),
        Some("Bearer secret-b") => Ok(OwnedPrincipal("owner-b".to_owned())),
        _ => Err(AuthError),
    }
}

fn owned_routes() -> Router {
    Router::new().route(
        "/work",
        post(|principal: Authenticated<OwnedPrincipal>| async move { principal.principal().0.clone() }),
    )
}

// A migrated selector derives exactly the keys the old one returned through
// `hash_for(..).into_unbound_subject_key()`, so stored counters carry over.
#[tokio::test]
async fn boundary_keys_match_the_native_derivation_the_old_selector_returned() {
    let running = running().await;
    let hasher = KeyHasher::new([9; 32]).unwrap();
    let owner = policy("owner", 1);
    let store = memory();
    // Spend owner-a's only request under the native derivation.
    let native = Check::new(hasher.hash_for(&owner, "owner-a"));
    assert!(store.check(&native).unwrap().permits_request());

    let client = HttpQuota::new(
        Quota::new(store),
        vec![owner],
        hasher,
        owner_from,
        |principal: &OwnedPrincipal, _: DirectPeer, subject| subject.derive(principal.0.as_str()),
    )
    .unwrap()
    .prepare(request_policy(&running, 1000), owned_routes())
    .in_process();

    let spent = client
        .request(request("/work", "Bearer secret-a"), peer())
        .await;
    assert_eq!(
        spent.status(),
        StatusCode::TOO_MANY_REQUESTS,
        "the boundary charges the counter the native derivation already spent"
    );
    let fresh = client
        .request(request("/work", "Bearer secret-b"), peer())
        .await;
    assert_eq!(
        fresh.status(),
        StatusCode::OK,
        "another subject has its own counter"
    );
    finish(running).await;
}
