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
    cleanup::CleanupBudget,
    lifecycle::{ShutdownBudget, Supervisor},
    operation::OperationContext,
};
use runlimit_core::{Check, FixedWindowPolicy, KeyHasher, PolicyId, ScopeId};
use runlimit_memory::{MemoryStore, MemoryStoreConfig};
use std::{convert::Infallible, time::Duration};

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let policy = FixedWindowPolicy::new(
        PolicyId::new("api.submit").unwrap(),
        ScopeId::new("owner").unwrap(),
        1,
        Duration::from_secs(60),
    )
    .unwrap();
    let hasher = KeyHasher::new([7; 32]).unwrap(); // Deterministic test secret only.
    let quota = Quota::new(MemoryStore::new(MemoryStoreConfig::new(100).unwrap()));
    let checks = [Check::new(&policy, hasher.hash_for(&policy, "owner-a"))];
    let context = OperationContext::new(Duration::from_secs(1)).unwrap();
    let result = quota
        .run(&context, Checks::new(&checks).unwrap(), |_scope| async {
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
    assert_eq!(allowances.len(), 1);
    assert_eq!(allowances.iter().next().unwrap().capacity(), 1);
    println!("native quota + work: passed");

    let second = Duration::from_secs(1);
    let cleanup = CleanupBudget::new(second, second, second).unwrap();
    let mut supervisor =
        Supervisor::new(ShutdownBudget::new(second, second, second, cleanup).unwrap());
    supervisor
        .register("component", |startup| async move {
            startup.acknowledge_started().draining().await;
            Ok(())
        })
        .unwrap();
    let running = supervisor.start();
    running.status().wait_ready().await.unwrap();
    let response_budget = ResponseConstructionBudget::new(second).unwrap();
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
        move |principal, _peer, policy| hasher.hash_for(policy, principal),
    )
    .unwrap();
    let probes = PublicProbes::new()
        .get("/live", || async { "live" })
        .unwrap();
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
    let probe = Request::builder().uri("/live").body(Body::empty()).unwrap();
    assert_eq!(
        client
            .request(probe, "127.0.0.1:32100".parse().unwrap())
            .await
            .status(),
        StatusCode::OK
    );
    let mut statuses = Vec::new();
    for _ in 0..2 {
        let request = Request::builder()
            .method("POST")
            .uri("/work")
            .header("authorization", "Bearer demonstration")
            .body(Body::empty())
            .unwrap();
        statuses.push(
            client
                .request(request, "127.0.0.1:32100".parse().unwrap())
                .await
                .status()
                .as_u16(),
        );
    }
    assert_eq!(statuses, [200, 429]);
    batter::lifecycle::check_shutdown(running.shutdown().await).unwrap();
    println!("protected HTTP status sequence: {statuses:?}");
}
