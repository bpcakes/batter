use axum::{
    Router,
    body::{Body, Bytes},
    routing::get,
};
use batter::{
    cleanup::{CleanupBudget, SkipReason},
    lifecycle::{Readiness, ShutdownBudget, Supervisor},
    operation::OperationContext,
    startup::Startup,
};
use batter_axum::register_http;
use std::{
    convert::Infallible,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::oneshot,
    time::timeout,
};

fn cleanup_budget() -> CleanupBudget {
    CleanupBudget::new(
        Duration::from_secs(1),
        Duration::from_secs(1),
        Duration::from_secs(1),
    )
    .unwrap()
}

fn supervisor() -> Supervisor {
    Supervisor::new(
        ShutdownBudget::new(
            Duration::from_millis(30),
            Duration::from_millis(30),
            Duration::from_secs(1),
            cleanup_budget(),
        )
        .unwrap(),
    )
}

async fn connect(address: std::net::SocketAddr) -> TcpStream {
    let mut client = TcpStream::connect(address).await.unwrap();
    client
        .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    client
}

#[tokio::test]
async fn owned_startup_serves_after_acknowledgement_then_drains_and_runs_cleanup() {
    let cleaned = Arc::new(AtomicBool::new(false));
    let cleanup = cleaned.clone();
    let (address_tx, address_rx) = oneshot::channel();
    let supervisor = supervisor();
    let handle = supervisor.handle();
    assert_eq!(handle.readiness(), Readiness::Starting);
    let mut starting = Startup::new(
        supervisor,
        OperationContext::new(Duration::from_secs(2)).unwrap(),
        cleanup_budget(),
        move |scope| {
            Box::pin(async move {
                scope.stage("bind").unwrap();
                let listener = TcpListener::bind("127.0.0.1:0").await?;
                let address = listener.local_addr()?;
                scope
                    .supervisor()
                    .on_cleanup("dependency", move || async move {
                        cleanup.store(true, Ordering::SeqCst);
                        Ok(())
                    })
                    .unwrap();
                register_http(
                    scope.supervisor(),
                    "http",
                    listener,
                    Router::new().route("/", get(|| async { "served" })),
                )
                .unwrap();
                address_tx.send(address).unwrap();
                Ok::<_, std::io::Error>(())
            })
        },
    )
    .start();
    let running = timeout(Duration::from_secs(3), starting.wait())
        .await
        .unwrap()
        .unwrap();
    timeout(Duration::from_secs(1), handle.wait_ready())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(handle.readiness(), Readiness::Ready);
    let address = address_rx.await.unwrap();
    let mut client = connect(address).await;
    let mut response = String::new();
    timeout(Duration::from_secs(2), client.read_to_string(&mut response))
        .await
        .unwrap()
        .unwrap();
    handle.request();
    let report = timeout(Duration::from_secs(2), running.wait())
        .await
        .unwrap()
        .unwrap();
    assert!(
        response.starts_with("HTTP/1.1 200 ") && response.ends_with("served"),
        "{response}"
    );
    assert!(report.is_success(), "{report:?}");
    assert_eq!(handle.readiness(), Readiness::Stopped);
    assert!(cleaned.load(Ordering::SeqCst));
    assert!(TcpStream::connect(address).await.is_err());
}

#[tokio::test]
async fn failed_registration_and_abandoned_startup_release_the_bound_listener() {
    let mut supervisor = supervisor();
    for name in ["", "http"] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let result = register_http(&mut supervisor, name, listener, Router::new());
        if name.is_empty() {
            assert!(result.is_err());
            let rebound = TcpListener::bind(address).await.unwrap();
            drop(rebound);
        } else {
            result.unwrap();
            assert_eq!(supervisor.handle().readiness(), Readiness::Starting);
            drop(supervisor);
            let rebound = TcpListener::bind(address).await.unwrap();
            drop(rebound);
            break;
        }
    }
}

struct StreamingBody {
    first: bool,
    release: oneshot::Receiver<()>,
    dropped: Option<oneshot::Sender<()>>,
}

impl http_body::Body for StreamingBody {
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<http_body::Frame<Bytes>, Infallible>>> {
        if self.first {
            self.first = false;
            return Poll::Ready(Some(Ok(http_body::Frame::data(Bytes::from_static(
                b"first",
            )))));
        }
        use std::future::Future;
        Pin::new(&mut self.release).poll(cx).map(|_| None)
    }
}

impl Drop for StreamingBody {
    fn drop(&mut self) {
        if let Some(dropped) = self.dropped.take() {
            let _ = dropped.send(());
        }
    }
}

#[tokio::test]
async fn streaming_outlives_response_budget_and_forced_wrapper_abort_skips_cleanup() {
    let (release_tx, release_rx) = oneshot::channel();
    let (dropped_tx, mut dropped_rx) = oneshot::channel();
    let body = Arc::new(std::sync::Mutex::new(Some(StreamingBody {
        first: true,
        release: release_rx,
        dropped: Some(dropped_tx),
    })));
    let mut supervisor = supervisor();
    let handle = supervisor.handle();
    let cleaned = Arc::new(AtomicBool::new(false));
    let cleanup = cleaned.clone();
    supervisor
        .on_cleanup("dependency", move || async move {
            cleanup.store(true, Ordering::SeqCst);
            Ok(())
        })
        .unwrap();
    let policy = batter_axum::RequestPolicy::new(handle.clone(), Duration::from_secs(1)).unwrap();
    let app = Router::new()
        .route(
            "/",
            get(move || async move { Body::new(body.lock().unwrap().take().unwrap()) }),
        )
        .route_layer(axum::middleware::from_fn_with_state(
            policy,
            batter_axum::request_admission,
        ))
        .layer(axum::middleware::from_fn(batter_axum::operational_http));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    register_http(&mut supervisor, "http", listener, app).unwrap();
    let running = supervisor.start();
    handle.mark_ready();
    timeout(Duration::from_secs(1), handle.wait_ready())
        .await
        .unwrap()
        .unwrap();
    let mut client = connect(address).await;
    let mut bytes = Vec::new();
    timeout(Duration::from_secs(2), async {
        while !bytes.windows(5).any(|chunk| chunk == b"first") {
            assert_ne!(client.read_buf(&mut bytes).await.unwrap(), 0);
        }
    })
    .await
    .unwrap();
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(2)).await;
    handle.request();
    let report = timeout(Duration::from_secs(2), running.wait())
        .await
        .unwrap()
        .unwrap();
    tokio::time::resume();
    let body_still_owned = matches!(
        dropped_rx.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    );
    // Release and await the native connection body even if the assertions below fail.
    let _ = release_tx.send(());
    timeout(Duration::from_secs(2), client.read_to_end(&mut bytes))
        .await
        .unwrap()
        .unwrap();
    timeout(Duration::from_secs(2), dropped_rx)
        .await
        .unwrap()
        .unwrap();
    assert!(String::from_utf8_lossy(&bytes).starts_with("HTTP/1.1 200 "));
    assert!(
        body_still_owned,
        "joining the wrapper did not establish a stopped body"
    );
    assert!(!report.is_success());
    assert!(report.forced_cancellation);
    assert_eq!(report.abort_requested, ["http"]);
    assert!(report.all_direct_tasks_joined());
    assert_eq!(report.cleanup.skipped.len(), 1);
    assert_eq!(report.cleanup.skipped[0].name, "dependency");
    assert_eq!(report.cleanup.skipped[0].reason, SkipReason::UnsafeTaskExit);
    assert!(!cleaned.load(Ordering::SeqCst));
}
