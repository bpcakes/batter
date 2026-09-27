//! The production executable with export enabled keeps its exit contract.

use crate::{
    PRIVATE,
    children::refused_address,
    collector::{Behavior, Collector, find, series},
};
use batter::telemetry::metrics as catalog;
use std::{
    process::{Command, Stdio},
    time::{Duration, Instant},
};

#[test]
fn production_executable_exports_startup_cleanup_without_changing_its_exit() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    let collector = runtime
        .block_on(Collector::start(Behavior::Accept))
        .unwrap();
    let database = runtime.block_on(refused_address());
    let mut child = Command::new(env!("CARGO_BIN_EXE_batter-example-reference-service"))
        .env_clear()
        .envs([
            (
                "DATABASE_URL",
                format!("postgres://user:{PRIVATE}@{database}/database?sslmode=disable"),
            ),
            ("JOBS_WORKER_ID", "metrics-worker".to_owned()),
            (
                "BATTER_AUTH_OWNER_ID",
                "00000000-0000-0000-0000-000000000001".to_owned(),
            ),
            ("BATTER_AUTH_TOKEN", "fake-metrics-token".to_owned()),
            ("BATTER_PROVIDER_BASE_URL", "http://127.0.0.1:9/".to_owned()),
            ("BATTER_PROVIDER_TOKEN", "fake-provider-token".to_owned()),
            ("BATTER_BIND", "127.0.0.1:0".to_owned()),
            ("BATTER_POOL_ACQUIRE_TIMEOUT_MS", "200".to_owned()),
            ("BATTER_METRICS_OTLP_ENDPOINT", collector.endpoint()),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() > deadline {
            child.kill().unwrap();
            let _ = child.wait();
            panic!("production executable did not settle");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    // Diagnostics are retained, never printed; the service failure line is unchanged.
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "Error: reference service failed\n"
    );
    let requests = collector.requests();
    assert_eq!(requests.len(), 1);
    let exported = series(&requests[0]);
    assert!(
        find(
            &exported,
            catalog::CLEANUP_HOOKS,
            &[("outcome", "succeeded")]
        )
        .is_some()
    );
    runtime.block_on(collector.close()).unwrap();
}
