//! Per-request cost of admitted-router dispatch as admitted routers are added.
//!
//! Each case assembles a boundary whose default group holds one native route
//! and `count` admitted routers, router `i` declaring only `/r{i}/items/{id}`,
//! and sends requests through the resulting `InProcessClient`: the native
//! route, the first and last admitted routers, and a path no route declares,
//! which the default group's fallback answers.
//!
//! `cargo bench -p batter-axum --bench declared_dispatch --locked` prints the
//! median nanoseconds per request. Without `--bench`, as under `cargo test`, it
//! sends each request once and checks only its status.

use axum::{
    Router,
    body::Body,
    extract::Path,
    http::{Request, StatusCode},
    routing::get,
};
use batter_axum::{
    GuardedRouter, HttpBoundary, InProcessClient, RequestPolicy, ResponseConstructionBudget,
    RouteInventory,
};
use batter_core::lifecycle::ShutdownHandle;
use std::{
    hint::black_box,
    time::{Duration, Instant},
};

const COUNTS: [usize; 3] = [1, 16, 64];
const SAMPLES: usize = 9;
const REQUESTS_PER_SAMPLE: u32 = 4_000;

fn admitted(index: usize) -> GuardedRouter {
    let pattern = format!("/r{index}/items/{{id}}");
    let router = Router::new().route(
        &pattern,
        get(|Path(id): Path<u32>| async move { id.to_string() }),
    );
    GuardedRouter::from_router(router, RouteInventory::new([pattern]).unwrap())
}

async fn client(handle: &ShutdownHandle, count: usize) -> InProcessClient {
    let routes = (0..count).fold(
        GuardedRouter::new().route("/native", get(|| async { "native" })),
        |routes, index| routes.merge(admitted(index)),
    );
    let budget = ResponseConstructionBudget::new(Duration::from_secs(5)).unwrap();
    HttpBoundary::new(RequestPolicy::new(handle.operation_admission(), budget))
        .assemble(routes)
        .await
        .unwrap()
        .in_process()
}

fn request(path: &str) -> Request<Body> {
    Request::builder().uri(path).body(Body::empty()).unwrap()
}

/// Median nanoseconds per request over the samples.
async fn measure(client: &InProcessClient, path: &str) -> u128 {
    let mut samples = Vec::with_capacity(SAMPLES);
    for _ in 0..REQUESTS_PER_SAMPLE {
        black_box(client.request(request(path)).await.status());
    }
    for _ in 0..SAMPLES {
        let started = Instant::now();
        for _ in 0..REQUESTS_PER_SAMPLE {
            black_box(client.request(request(path)).await.status());
        }
        samples.push(started.elapsed().as_nanos() / u128::from(REQUESTS_PER_SAMPLE));
    }
    samples.sort_unstable();
    samples[SAMPLES / 2]
}

async fn run(bench: RunMode) {
    let bench = bench == RunMode::Benchmark;
    let (handle, approval) = ShutdownHandle::new_with_readiness_approval();
    approval.approve();
    if bench {
        println!("| admitted routers | native | first match | last match | miss |");
        println!("| ---: | ---: | ---: | ---: | ---: |");
    }
    for count in COUNTS {
        let client = client(&handle, count).await;
        let last = format!("/r{}/items/7", count - 1);
        let cases = [
            ("/native", StatusCode::OK),
            ("/r0/items/7", StatusCode::OK),
            (last.as_str(), StatusCode::OK),
            ("/missing/7", StatusCode::NOT_FOUND),
        ];
        if !bench {
            for (path, status) in cases {
                assert_eq!(
                    client.request(request(path)).await.status(),
                    status,
                    "{path}"
                );
            }
            continue;
        }
        let mut row = format!("| {count} |");
        for (path, _) in cases {
            row.push_str(&format!(" {} ns |", measure(&client, path).await));
        }
        println!("{row}");
    }
}

fn main() {
    let bench = std::env::args().any(|argument| argument == "--bench");
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(run(if bench {
            RunMode::Benchmark
        } else {
            RunMode::Smoke
        }));
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RunMode {
    Benchmark,
    Smoke,
}
