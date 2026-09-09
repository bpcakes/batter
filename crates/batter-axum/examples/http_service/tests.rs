#[path = "../../tests/support/capture.rs"]
mod capture;

use super::{router, support};
use batter::lifecycle::Supervisor;
use capture::Capture;
use std::{
    collections::HashSet,
    io::{self, Read, Write},
    net::{SocketAddr, TcpStream},
    time::Duration,
};
use tokio::{sync::oneshot, time::timeout};

async fn get(address: SocketAddr) -> io::Result<String> {
    tokio::task::spawn_blocking(move || {
        let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(2))?;
        stream.set_read_timeout(Some(Duration::from_secs(2)))?;
        stream.set_write_timeout(Some(Duration::from_secs(2)))?;
        stream.write_all(b"GET /ready HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")?;
        let mut response = String::new();
        stream.take(16 * 1024).read_to_string(&mut response)?;
        Ok(response)
    })
    .await
    .map_err(io::Error::other)?
}

#[test]
fn actual_router_readiness_policy_is_observed_over_http_in_each_lifecycle_phase() {
    assert_readiness(
        Capture::new(),
        [Some("INFO"), Some("INFO"), Some("INFO"), Some("WARN")],
    );
}

#[test]
fn actual_router_readiness_filter_keeps_stopped_failure_identity_and_suppresses_info() {
    assert_readiness(
        Capture::with_filter("info,batter=warn"),
        [None, None, None, Some("WARN")],
    );
}

async fn readiness_responses() -> [String; 4] {
    let mut supervisor = Supervisor::new(support::shutdown_budget());
    let handle = supervisor.handle();
    let (started_tx, started_rx) = oneshot::channel();
    let (release_tx, release_rx) = oneshot::channel();
    let mut release = Some(release_tx);
    supervisor
        .register("phase-control", move |shutdown| async move {
            shutdown.mark_started();
            let _ = started_tx.send(());
            shutdown.draining().await;
            let _ = release_rx.await;
            Ok(())
        })
        .unwrap();
    let app = router(handle.clone(), Duration::from_secs(1)).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (stop_tx, stop_rx) = oneshot::channel();
    // This listener remains available to inspect all phases of the actual
    // example router. It does not model the binary's connection shutdown.
    let mut server = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = stop_rx.await;
            })
            .await
    });
    let running = supervisor.start();
    let result: Result<_, Box<dyn std::error::Error + Send + Sync>> = async {
        timeout(Duration::from_secs(2), started_rx).await??;
        let starting = get(address).await?;
        handle.mark_ready();
        let ready = get(address).await?;
        handle.request();
        let draining = get(address).await?;
        let _ = release.take().unwrap().send(());
        let report = timeout(Duration::from_secs(5), running.wait()).await??;
        let stopped = get(address).await?;
        Ok(([starting, ready, draining, stopped], report.is_success()))
    }
    .await;

    // Teardown precedes assertions, including on a failed client request.
    handle.request();
    if let Some(release) = release.take() {
        let _ = release.send(());
    }
    let report = timeout(Duration::from_secs(5), running.wait()).await;
    let _ = stop_tx.send(());
    let server_result = timeout(Duration::from_secs(5), &mut server).await;
    if server_result.is_err() {
        server.abort();
        let _ = server.await;
    }
    assert!(
        matches!(&result, Ok((_, true)))
            && matches!(&report, Ok(Ok(report)) if report.is_success())
            && matches!(&server_result, Ok(Ok(Ok(())))),
        "request exercise: {result:?}\nsupervisor teardown: {report:?}\nHTTP server teardown: {server_result:?}"
    );
    result.unwrap().0
}

fn assert_readiness(capture: Capture, levels: [Option<&str>; 4]) {
    let responses = capture.block_on(readiness_responses());
    let text = capture.text();
    let events: Vec<_> = text
        .lines()
        .filter(|line| line.contains("HTTP response boundary finished"))
        .collect();
    assert_eq!(events.len(), levels.iter().flatten().count(), "{text}");
    let mut ids = HashSet::new();
    for ((response, level), (status, outcome)) in responses.iter().zip(levels).zip([
        (503, "server_error"),
        (200, "completed"),
        (503, "server_error"),
        (503, "server_error"),
    ]) {
        assert!(
            response.starts_with(&format!("HTTP/1.1 {status} ")),
            "{response}"
        );
        let id = response
            .lines()
            .find_map(|line| line.strip_prefix("x-request-id: "))
            .unwrap()
            .trim();
        assert!(id.starts_with("example-"), "{response}");
        assert!(ids.insert(id), "duplicate response identity: {id}");
        let matching: Vec<_> = events
            .iter()
            .filter(|event| event.contains(&format!("request_id={id}}}")))
            .collect();
        assert_eq!(matching.len(), usize::from(level.is_some()), "{text}");
        let Some(level) = level else { continue };
        let event = matching[0];
        assert!(event.trim_start().starts_with(level), "{text}");
        let (_, fields) = event.split_once("HTTP response boundary finished").unwrap();
        assert!(fields.contains(&format!("status={status}")), "{text}");
        assert!(
            fields.contains(&format!("http_outcome=\"{outcome}\"")),
            "{text}"
        );
        assert!(fields.contains("route=\"/ready\""), "{text}");
    }
}
