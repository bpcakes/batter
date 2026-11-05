//! Actual compiled public usage, without the test helpers or a reference service.
use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
    routing::post,
};
use batter::axum::{RequestPolicy, ResponseConstructionBudget};
use batter::runlimit::{
    http::{AuthInput, Authenticated, HttpQuota, PublicProbes},
    quota::{Admission, Checks, Quota, RunResult},
};
use batter::{
    BoxError,
    cleanup::CleanupBudget,
    lifecycle::{ShutdownBudget, Supervisor},
};
use runlimit_core::{Check, FixedWindowPolicy, KeyHasher, PolicyId, ScopeId};
use runlimit_memory::{MemoryStore, MemoryStoreConfig};
use std::{convert::Infallible, net::SocketAddr, time::Duration};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), BoxError> {
    // First statement, unwrapped: prints each panic's location, never its payload.
    std::panic::set_hook(batter::redacting_panic_hook());
    let policy = FixedWindowPolicy::new(
        PolicyId::new("api.submit")?,
        ScopeId::new("owner")?,
        1,
        Duration::from_secs(60),
    )?;
    let hasher = KeyHasher::new([7; 32])?; // Deterministic test secret only.
    let quota = Quota::new(MemoryStore::new(MemoryStoreConfig::new(100)?));
    demonstrate_native_admission(&quota, &policy, &hasher).await?;

    let second = Duration::from_secs(1);
    let cleanup = CleanupBudget::new(second, second, second)?;
    let mut supervisor = Supervisor::new(ShutdownBudget::new(second, second, second, cleanup)?);
    supervisor.register("component", |startup| async move {
        let running = startup.acknowledge_started();
        running.draining().await;
        Ok(running.stopped())
    })?;
    let running = supervisor.start();
    running
        .status()
        .wait_ready()
        .await
        .map_err(|_| std::io::Error::other("startup failed"))?;
    let response_budget = ResponseConstructionBudget::new(second)?;
    let request_policy = RequestPolicy::new(running.operation_admission(), response_budget);
    // Authentication returns the principal; selection never reads an untrusted extension.
    let boundary = HttpQuota::new(
        quota,
        vec![policy],
        |input: AuthInput| async move {
            if input
                .headers
                .get("authorization")
                .is_some_and(|value| value == "Bearer demonstration")
            {
                Ok("owner-b")
            } else {
                Err(std::io::Error::other("authentication failed"))
            }
        },
        move |principal, _peer, policy| {
            hasher
                .hash_for(policy, principal)
                .into_unbound_subject_key()
        },
    )?;
    let probes = PublicProbes::new().get("/live", || async { "live" })?;
    let prepared = boundary.with_public_probes(probes).prepare(
        request_policy,
        Router::new().route(
            "/work",
            post(|principal: Authenticated<&'static str>| async move {
                (*principal.principal()).to_owned()
            }),
        ),
    );
    // Production uses prepared.register_in(scope, listener). This finite demo uses its explicit test transport.
    let client = prepared.in_process();
    let peer: SocketAddr = "127.0.0.1:32100".parse()?;
    let probe = Request::builder().uri("/live").body(Body::empty())?;
    assert_eq!(
        client.request(probe, peer).await.status(),
        StatusCode::OK,
        "public probes bypass authentication and quota"
    );
    let mut statuses = Vec::new();
    for _ in 0..2 {
        let request = Request::builder()
            .method("POST")
            .uri("/work")
            .header("authorization", "Bearer demonstration")
            .body(Body::empty())?;
        statuses.push(client.request(request, peer).await.status().as_u16());
    }
    assert_eq!(
        statuses,
        [200, 429],
        "the one-request window admits the first call and denies the second"
    );
    running.shutdown_checked().await?;
    println!("protected HTTP status sequence: {statuses:?}");
    Ok(())
}

async fn demonstrate_native_admission(
    quota: &Quota<MemoryStore>,
    policy: &FixedWindowPolicy,
    hasher: &KeyHasher,
) -> Result<(), BoxError> {
    let checks = [Check::new(hasher.hash_for(policy, "owner-a"))];
    let context = batter::operation::OperationOwner::new(Duration::from_secs(1))?.into_context();
    let result = quota
        .run(&context, Checks::new(&checks)?, |_scope| async {
            Ok::<_, Infallible>(42)
        })
        .await;
    let RunResult::Admitted {
        admission: Admission::Allowed { allowances },
        work: Ok(42),
    } = result
    else {
        panic!("expected native admission and work");
    };
    assert_eq!(allowances.len(), 1, "one check yields one allowance");
    let allowance = allowances
        .iter()
        .next()
        .ok_or("admission returned no allowance")?;
    assert_eq!(
        allowance.capacity().get(),
        1,
        "the fixed window admits one request"
    );
    println!("native quota + work: passed");
    Ok(())
}
