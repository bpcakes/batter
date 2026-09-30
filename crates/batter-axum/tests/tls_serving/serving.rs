//! The canonical assembled boundary served over an application-owned listener.
//!
//! These cases repeat the TCP serving contract across a real rustls listener:
//! transferred listener ownership, startup acknowledgement before readiness,
//! the listener's own peer metadata, complete graceful draining of a connection
//! that is still open when drain begins, and destruction of an accept that
//! never produced a connection.

use super::support::{self, Accepting, TlsListener, WAIT};
use axum::{
    body::{Body, Bytes},
    extract::ConnectInfo,
    routing::get,
};
use batter_axum::{
    AssembledHttp, GuardedRouter, HttpBoundary, ProbePath, RequestPolicy,
    ResponseConstructionBudget,
};
use batter_core::{
    RegistrationError,
    cleanup::CleanupBudget,
    lifecycle::{Readiness, ShutdownBudget, ShutdownHandle, Supervisor},
    startup::Startup,
};
use std::{
    convert::Infallible,
    future::{Future, poll_fn},
    net::SocketAddr,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::oneshot,
    time::timeout,
};

fn cleanup_budget() -> CleanupBudget {
    CleanupBudget::new(WAIT, WAIT, WAIT).unwrap()
}

fn supervisor() -> Supervisor {
    Supervisor::new(ShutdownBudget::new(WAIT, WAIT, WAIT, cleanup_budget()).unwrap())
}

fn request_context() -> batter_core::operation::OperationContext {
    batter_core::operation::OperationOwner::new(WAIT)
        .unwrap()
        .into_context()
}

/// A response body the test releases, so a connection stays busy across drain.
struct HeldBody {
    sent_first: bool,
    sent_last: bool,
    release: oneshot::Receiver<()>,
}

impl http_body::Body for HeldBody {
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<http_body::Frame<Bytes>, Infallible>>> {
        if !self.sent_first {
            self.sent_first = true;
            return Poll::Ready(Some(Ok(http_body::Frame::data(Bytes::from_static(
                b"first",
            )))));
        }
        if self.sent_last {
            return Poll::Ready(None);
        }
        match Pin::new(&mut self.release).poll(cx) {
            Poll::Ready(_) => {
                self.sent_last = true;
                Poll::Ready(Some(Ok(http_body::Frame::data(Bytes::from_static(
                    b"last",
                )))))
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

/// The canonical library-owned composition: one probe outside admission and
/// guarded routes inside it.
async fn boundary(handle: &ShutdownHandle, held: Option<HeldBody>) -> AssembledHttp {
    let held = Arc::new(Mutex::new(held));
    let guarded = GuardedRouter::new()
        .route(
            "/peer",
            get(|ConnectInfo(peer): ConnectInfo<SocketAddr>| async move { peer.to_string() }),
        )
        .route(
            "/stream",
            get(move || async move { Body::new(held.lock().unwrap().take().unwrap()) }),
        );
    HttpBoundary::new(RequestPolicy::new(
        handle.operation_admission(),
        ResponseConstructionBudget::new(WAIT).unwrap(),
    ))
    .with_liveness(ProbePath::new("/live").unwrap())
    .unwrap()
    .assemble(guarded)
    .await
    .unwrap()
}

fn request(path: &str, close: bool) -> Vec<u8> {
    let connection = if close { "close" } else { "keep-alive" };
    format!(
        "GET {path} HTTP/1.1\r\nHost: {}\r\nConnection: {connection}\r\n\
         Forwarded: for=192.0.2.40:4567\r\nX-Forwarded-For: 192.0.2.41\r\n\r\n",
        support::SERVER_NAME
    )
    .into_bytes()
}

/// A probe outside admission answers over the same TLS listener.
async fn assert_probe(client_config: &rustls::ClientConfig, address: SocketAddr) {
    let mut probe = support::connect(client_config, address).await;
    probe.write_all(&request("/live", true)).await.unwrap();
    let mut live = String::new();
    timeout(WAIT, probe.read_to_string(&mut live))
        .await
        .unwrap()
        .unwrap();
    assert!(live.starts_with("HTTP/1.1 200 "), "{live}");
}

/// A completed guarded request carries the listener's own transport peer.
/// The expected address and port come from the client, never from the server.
async fn assert_peer(client_config: &rustls::ClientConfig, address: SocketAddr) {
    let mut client = support::connect(client_config, address).await;
    let expected = support::client_address(&client);
    client.write_all(&request("/peer", true)).await.unwrap();
    let mut answered = String::new();
    timeout(WAIT, client.read_to_string(&mut answered))
        .await
        .unwrap()
        .unwrap();
    let (headers, peer) = answered.split_once("\r\n\r\n").unwrap();
    assert!(headers.starts_with("HTTP/1.1 200 "), "{answered}");
    assert_eq!(peer, expected.to_string());
}

#[tokio::test]
async fn assembled_boundary_over_tls_acknowledges_serves_the_peer_and_drains_an_open_connection() {
    let (server_config, client_config) = support::certificates();
    let supervisor = supervisor();
    let handle = supervisor.handle();
    let assembling = handle.clone();
    let (address_tx, address_rx) = oneshot::channel();
    let (cleanup_tx, mut cleanup_rx) = oneshot::channel();
    let (release_tx, release) = oneshot::channel();
    let mut starting = Startup::scoped(
        supervisor,
        request_context(),
        cleanup_budget(),
        move |scope| {
            Box::pin(async move {
                scope.stage("bind").unwrap();
                let (listener, _observations) = TlsListener::bind(server_config).await;
                let address = listener.address();
                scope
                    .reserve_cleanup("dependency")
                    .unwrap()
                    .register(move || async move {
                        // Cleanup follows release of the serving listener.
                        let rebound = TcpListener::bind(address).await?;
                        cleanup_tx.send(()).unwrap();
                        drop(rebound);
                        Ok(())
                    });
                let held = HeldBody {
                    sent_first: false,
                    sent_last: false,
                    release,
                };
                boundary(&assembling, Some(held))
                    .await
                    .register_with_connect_info_in(scope, "http", listener)
                    .unwrap();
                address_tx.send(address).unwrap();
                Ok::<_, std::io::Error>(())
            })
        },
    )
    .start();
    let running = timeout(WAIT, starting.wait()).await.unwrap().unwrap();
    timeout(WAIT, handle.status().wait_ready())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(handle.status().readiness(), Readiness::Ready);
    let address = address_rx.await.unwrap();
    assert_probe(&client_config, address).await;
    assert_peer(&client_config, address).await;

    // A further connection is still serving a response when drain begins.
    let mut open = support::connect(&client_config, address).await;
    open.write_all(&request("/stream", false)).await.unwrap();
    let mut streamed = Vec::new();
    timeout(WAIT, async {
        while !streamed.windows(5).any(|chunk| chunk == b"first") {
            assert_ne!(open.read_buf(&mut streamed).await.unwrap(), 0);
        }
    })
    .await
    .unwrap();

    handle.request();
    // The listener is released before connection completion is awaited.
    timeout(WAIT, async {
        while TcpStream::connect(address).await.is_ok() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let mut completion = Box::pin(running.wait());
    assert!(
        poll_fn(|cx| Poll::Ready(completion.as_mut().poll(cx)))
            .await
            .is_pending(),
        "drain must wait for the connection that is still open"
    );
    assert!(matches!(
        cleanup_rx.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    ));

    release_tx.send(()).unwrap();
    timeout(WAIT, open.read_to_end(&mut streamed))
        .await
        .unwrap()
        .unwrap();
    let served = String::from_utf8(streamed).unwrap();
    assert!(served.starts_with("HTTP/1.1 200 "), "{served}");
    assert!(
        served.contains("first") && served.contains("last"),
        "{served}"
    );

    let report = timeout(WAIT, completion).await.unwrap().unwrap();
    assert!(report.is_success(), "{report:?}");
    assert!(report.all_direct_tasks_joined());
    assert_eq!(report.cleanup.records.len(), 1);
    assert_eq!(report.cleanup.records[0].name, "dependency");
    timeout(WAIT, cleanup_rx).await.unwrap().unwrap();
    assert_eq!(handle.status().readiness(), Readiness::Stopped);
    assert!(TcpStream::connect(address).await.is_err());
}

#[tokio::test]
async fn drain_releases_an_accept_parked_in_its_handshake_and_still_stops_cleanly() {
    let (server_config, _client_config) = support::certificates();
    let mut supervisor = supervisor();
    let handle = supervisor.handle();
    let (listener, mut observations) = TlsListener::bind(server_config).await;
    let address = listener.address();
    boundary(&handle, None)
        .await
        .register_in(&mut supervisor, "http", listener)
        .unwrap();
    let running = supervisor.start();
    timeout(WAIT, handle.status().wait_ready())
        .await
        .unwrap()
        .unwrap();

    // A transport connection that never sends a client hello parks the
    // application's own handshake inside `accept`.
    let parked = TcpStream::connect(address).await.unwrap();
    assert_eq!(
        timeout(WAIT, observations.recv()).await.unwrap(),
        Some(Accepting::HandshakeStarted)
    );

    handle.request();
    assert_eq!(
        timeout(WAIT, observations.recv()).await.unwrap(),
        Some(Accepting::PendingReleased),
        "drain must destroy the accept in progress"
    );
    let report = timeout(WAIT, running.wait()).await.unwrap().unwrap();
    assert!(report.is_success(), "{report:?}");
    assert!(report.all_direct_tasks_joined());
    assert_eq!(handle.status().readiness(), Readiness::Stopped);
    drop(parked);
    drop(TcpListener::bind(address).await.unwrap());
}

#[tokio::test]
async fn canonical_registration_releases_rejected_and_abandoned_custom_listeners() {
    let (server_config, _client_config) = support::certificates();
    let mut supervisor = supervisor();
    let handle = supervisor.handle();
    let (rejected, _observations) = TlsListener::bind(server_config.clone()).await;
    let rejected_address = rejected.address();
    assert!(matches!(
        boundary(&handle, None)
            .await
            .register_in(&mut supervisor, "", rejected),
        Err(RegistrationError::InvalidName)
    ));
    drop(TcpListener::bind(rejected_address).await.unwrap());

    let (accepted, _observations) = TlsListener::bind(server_config).await;
    let accepted_address = accepted.address();
    boundary(&handle, None)
        .await
        .register_with_connect_info_in(&mut supervisor, "http", accepted)
        .unwrap();
    assert!(TcpListener::bind(accepted_address).await.is_err());
    assert_eq!(supervisor.status().readiness(), Readiness::Starting);
    drop(supervisor);
    drop(TcpListener::bind(accepted_address).await.unwrap());
}
