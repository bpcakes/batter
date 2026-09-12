use axum::{
    Router,
    body::{Body, Bytes},
    routing::get,
};
use batter::{
    cleanup::{CleanupBudget, SkipReason},
    lifecycle::{Readiness, ShutdownBudget, Supervisor},
    operation::OperationContext,
    startup::{Startup, StartupCause, StartupError, StartupOutcome},
};
use batter_axum::{register_http, register_http_in, register_http_with_connect_info_in};
use std::{
    convert::Infallible,
    ops::{Deref, DerefMut},
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

type RegisterHttp =
    fn(&mut Supervisor, &'static str, TcpListener, Router) -> Result<(), batter::RegistrationError>;

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

struct SupervisorWrapper(Supervisor);
impl Deref for SupervisorWrapper {
    type Target = Supervisor;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl DerefMut for SupervisorWrapper {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
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
    let mut starting = Startup::scoped(
        supervisor,
        OperationContext::new(Duration::from_secs(2)).unwrap(),
        cleanup_budget(),
        move |scope| {
            Box::pin(async move {
                scope.stage("bind").unwrap();
                let listener = TcpListener::bind("127.0.0.1:0").await?;
                let address = listener.local_addr()?;
                scope
                    .reserve_cleanup("dependency")
                    .unwrap()
                    .register(move || async move {
                        cleanup.store(true, Ordering::SeqCst);
                        Ok(())
                    });
                register_http_in(
                    scope,
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
async fn invalid_registration_and_unstarted_supervisor_drop_release_the_bound_listener() {
    let _: fn(
        &mut Supervisor,
        &'static str,
        TcpListener,
        Router,
    ) -> Result<(), batter::RegistrationError> = register_http;
    let mut wrapped = SupervisorWrapper(supervisor());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    assert!(register_http(&mut wrapped, "", listener, Router::new()).is_err());

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
    streaming_abort(register_http).await;
}

#[tokio::test]
async fn peer_server_forced_wrapper_abort_still_skips_dependent_cleanup() {
    streaming_abort(register_http_with_connect_info_in).await;
}

async fn streaming_abort(register: RegisterHttp) {
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
    register(&mut supervisor, "http", listener, app).unwrap();
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

#[tokio::test]
async fn borrowed_startup_waiter_is_inert_but_owner_drop_releases_listener_before_cleanup() {
    startup_abandonment(register_http).await;
}

#[tokio::test]
async fn peer_server_startup_owner_drop_releases_listener_before_cleanup() {
    startup_abandonment(register_http_with_connect_info_in).await;
}

async fn startup_abandonment(register: RegisterHttp) {
    use std::future::{Future, poll_fn};
    let base = supervisor();
    let handle = base.handle();
    let (bound_tx, bound_rx) = oneshot::channel();
    let (cleanup_tx, mut cleanup_rx) = oneshot::channel();
    let (release_tx, release_rx) = oneshot::channel();
    let mut starting = Startup::new(
        base,
        OperationContext::new(Duration::from_secs(10)).unwrap(),
        cleanup_budget(),
        move |scope| {
            Box::pin(async move {
                scope.stage("registered").unwrap();
                let listener = TcpListener::bind("127.0.0.1:0").await?;
                let address = listener.local_addr()?;
                scope
                    .supervisor()
                    .on_cleanup("dependency", move || async move {
                        // This fails if startup retains the registered server capture
                        // while closing a resource that the server could still use.
                        let rebound = TcpListener::bind(address).await?;
                        cleanup_tx.send(()).unwrap();
                        release_rx.await.unwrap();
                        drop(rebound);
                        Ok(())
                    })
                    .unwrap();
                register(scope.supervisor(), "http", listener, Router::new()).unwrap();
                bound_tx.send(address).unwrap();
                std::future::pending::<Result<(), std::io::Error>>().await
            })
        },
    )
    .start();
    let observer = starting.observer();
    let address = timeout(Duration::from_secs(2), bound_rx)
        .await
        .unwrap()
        .unwrap();
    let mut waiter = Box::pin(starting.wait());
    assert!(
        poll_fn(|cx| Poll::Ready(waiter.as_mut().poll(cx)))
            .await
            .is_pending()
    );
    drop(waiter);
    tokio::task::yield_now().await;
    assert_eq!(handle.readiness(), Readiness::Starting);
    assert!(matches!(
        cleanup_rx.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    ));
    assert!(
        TcpListener::bind(address).await.is_err(),
        "borrowed waiter cannot release the listener"
    );

    drop(starting);
    timeout(Duration::from_secs(2), &mut cleanup_rx)
        .await
        .unwrap()
        .unwrap();
    let mut completion = Box::pin(observer.wait());
    assert!(
        poll_fn(|cx| Poll::Ready(completion.as_mut().poll(cx)))
            .await
            .is_pending(),
        "report must wait for cleanup"
    );
    release_tx.send(()).unwrap();
    let outcome = timeout(Duration::from_secs(2), completion).await.unwrap();
    assert_startup_drain(outcome);
    assert_eq!(handle.readiness(), Readiness::Draining);
    drop(TcpListener::bind(address).await.unwrap());
}

fn assert_startup_drain(outcome: StartupOutcome<std::io::Error>) {
    let StartupOutcome::Failed(StartupError::Failed(report)) = outcome else {
        panic!("expected retained startup drain failure")
    };
    assert!(matches!(report.cause, StartupCause::Draining));
    assert_eq!(report.stage, "registered");
    assert!(report.cleanup.is_success());
    assert_eq!(report.cleanup.records.len(), 1);
    assert_eq!(report.cleanup.records[0].name, "dependency");
}

#[tokio::test]
async fn duplicate_http_registration_releases_only_the_rejected_listener() {
    let mut supervisor = supervisor();
    let first = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let first_address = first.local_addr().unwrap();
    register_http(&mut supervisor, "http", first, Router::new()).unwrap();
    let second = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let second_address = second.local_addr().unwrap();
    assert!(register_http(&mut supervisor, "http", second, Router::new()).is_err());
    drop(TcpListener::bind(second_address).await.unwrap());
    assert!(TcpListener::bind(first_address).await.is_err());
    drop(supervisor);
    drop(TcpListener::bind(first_address).await.unwrap());
}
