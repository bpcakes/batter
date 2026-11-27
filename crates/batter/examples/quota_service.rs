//! Actual compiled public usage, without the test helpers or a reference service.
mod support;

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
    routing::post,
};
use batter::axum::{RequestPolicy, ResponseConstructionBudget};
use batter::runlimit::{
    http::{AuthInput, Authenticated, HttpQuota, PublicProbes},
    memory::{MemoryStore, MemoryStoreConfig},
    native::{Check, FixedWindowPolicy, KeyHasher, PolicyId, ScopeId},
    quota::{Admission, Checks, Quota, RunResult},
};
use batter::{BoxError, lifecycle::Supervisor};
use std::{convert::Infallible, net::SocketAddr, time::Duration};
use support::complete;

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

    let mut supervisor = Supervisor::new(support::shutdown_budget());
    supervisor.register("component", |startup| async move {
        let running = startup.acknowledge_started();
        running.draining().await;
        Ok(running.stopped())
    })?;
    let running = supervisor.start();
    // Keep the body's failures inside this block as errors, never panics, so
    // they cannot leave main before the supervisor has shut down; both
    // outcomes reach `complete`, and the result is reported afterwards.
    let mut statuses = Vec::new();
    let served = async {
        running
            .status()
            .wait_ready()
            .await
            .map_err(|_| std::io::Error::other("startup failed"))?;
        let response_budget = ResponseConstructionBudget::new(Duration::from_secs(1))?;
        let request_policy = RequestPolicy::new(running.operation_admission(), response_budget);
        // Authentication returns the principal; the boundary derives each
        // policy's key from the material the selector returns.
        let boundary = HttpQuota::new(
            quota,
            vec![policy],
            hasher,
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
            |principal: &&'static str, _peer, _policy: &FixedWindowPolicy| *principal,
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
        if client.request(probe, peer).await.status() != StatusCode::OK {
            return Err("public probes bypass authentication and quota".into());
        }
        for _ in 0..2 {
            let request = Request::builder()
                .method("POST")
                .uri("/work")
                .header("authorization", "Bearer demonstration")
                .body(Body::empty())?;
            statuses.push(client.request(request, peer).await.status().as_u16());
        }
        if statuses != [200, 429] {
            return Err(
                "the one-request window admits the first call and denies the second".into(),
            );
        }
        Ok::<_, BoxError>(())
    }
    .await;
    complete(served, running.shutdown_checked().await)?;
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
