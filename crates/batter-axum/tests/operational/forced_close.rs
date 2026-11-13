//! Connections that outlive drain: the registered server closes their I/O
//! part-way through the cancel allowance, so the wrapper is joined and
//! dependent cleanup runs. Only a zero cancel allowance, or a connection task
//! that never yields, still reaches abort.

use axum::{
    Router,
    body::{Body, Bytes},
    routing::get,
};
use batter_axum::{
    ResponseConstructionBudget,
    low_level::{register_http, register_http_with_connect_info_in},
};
use batter_core::{
    BoxError,
    cleanup::{CleanupBudget, SkipReason},
    lifecycle::{
        ManagedComponent, ManagedSettlement, SharedShutdownReport, ShutdownBudget, ShutdownHandle,
        ShutdownReport, Supervisor,
    },
    operation::OperationOwner,
};
use std::{
    convert::Infallible,
    future::Future,
    pin::Pin,
    sync::{
        Arc, Mutex,
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

type RegisterHttp = fn(
    &mut Supervisor,
    &'static str,
    TcpListener,
    Router,
) -> Result<(), batter_core::RegistrationError>;

const WAIT: Duration = Duration::from_secs(5);
const DRAIN: Duration = Duration::from_millis(50);
const CANCEL: Duration = Duration::from_millis(400);

/// A served router with one dependency finalizer behind it.
struct Served {
    handle: ShutdownHandle,
    address: std::net::SocketAddr,
    cleaned: Arc<AtomicBool>,
    running: batter_core::lifecycle::RunningSupervisor,
}

impl Served {
    async fn start(
        register: RegisterHttp,
        cancel: Duration,
        routes: impl FnOnce(Router) -> Router,
    ) -> Self {
        Self::start_with(register, cancel, routes, |_| {}).await
    }

    /// Like `start`, with other components registered beside the server.
    async fn start_with(
        register: RegisterHttp,
        cancel: Duration,
        routes: impl FnOnce(Router) -> Router,
        components: impl FnOnce(&mut Supervisor),
    ) -> Self {
        let cleanup = CleanupBudget::new(WAIT, WAIT, WAIT).unwrap();
        let mut supervisor =
            Supervisor::new(ShutdownBudget::new(DRAIN, cancel, WAIT, cleanup).unwrap());
        components(&mut supervisor);
        let handle = supervisor.handle();
        let cleaned = Arc::new(AtomicBool::new(false));
        let finalized = cleaned.clone();
        supervisor
            .on_cleanup("dependency", move || async move {
                finalized.store(true, Ordering::SeqCst);
                Ok(())
            })
            .unwrap();
        let policy = batter_axum::RequestPolicy::new(
            handle.operation_admission(),
            ResponseConstructionBudget::new(WAIT).unwrap(),
        );
        let app = routes(Router::new())
            .route_layer(axum::middleware::from_fn_with_state(
                policy,
                batter_axum::low_level::request_admission,
            ))
            .layer(axum::middleware::from_fn(
                batter_axum::low_level::operational_http,
            ));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        register(&mut supervisor, "http", listener, app).unwrap();
        let running = supervisor.start();
        timeout(WAIT, handle.status().wait_ready())
            .await
            .unwrap()
            .unwrap();
        Self {
            handle,
            address,
            cleaned,
            running,
        }
    }

    async fn request(&self, head: &[u8]) -> TcpStream {
        let mut client = TcpStream::connect(self.address).await.unwrap();
        client.write_all(head).await.unwrap();
        client
    }

    async fn shut_down(&self) -> SharedShutdownReport {
        self.handle.request();
        timeout(WAIT, self.running.wait()).await.unwrap().unwrap()
    }

    /// The wrapper returned on its own and the dependency finalizer ran.
    fn assert_joined_and_cleaned(&self, report: &ShutdownReport) {
        assert!(report.is_success(), "{report:?}");
        assert!(report.abort_requested.is_empty(), "{report:?}");
        assert!(report.all_direct_tasks_joined());
        assert!(report.cleanup.skipped.is_empty(), "{report:?}");
        assert_eq!(report.cleanup.records.len(), 1);
        assert!(self.cleaned.load(Ordering::SeqCst));
    }
}

const GET: &[u8] = b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n";

/// One frame, then pending until the test releases it or it is destroyed.
struct HeldBody {
    first: bool,
    release: oneshot::Receiver<()>,
    dropped: Option<oneshot::Sender<()>>,
}

impl http_body::Body for HeldBody {
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
        Pin::new(&mut self.release).poll(cx).map(|_| None)
    }
}

impl Drop for HeldBody {
    fn drop(&mut self) {
        if let Some(dropped) = self.dropped.take() {
            let _ = dropped.send(());
        }
    }
}

struct Held {
    release: oneshot::Sender<()>,
    dropped: oneshot::Receiver<()>,
}

fn held_route(router: Router) -> (Router, Held) {
    let (release, released) = oneshot::channel();
    let (dropped_tx, dropped) = oneshot::channel();
    let body = Arc::new(Mutex::new(Some(HeldBody {
        first: true,
        release: released,
        dropped: Some(dropped_tx),
    })));
    let router = router.route(
        "/",
        get(move || async move { Body::new(body.lock().unwrap().take().unwrap()) }),
    );
    (router, Held { release, dropped })
}

async fn read_first_frame(client: &mut TcpStream) -> Vec<u8> {
    let mut bytes = Vec::new();
    timeout(WAIT, async {
        while !bytes.windows(5).any(|chunk| chunk == b"first") {
            assert_ne!(client.read_buf(&mut bytes).await.unwrap(), 0);
        }
    })
    .await
    .unwrap();
    bytes
}

#[tokio::test]
async fn pending_body_is_destroyed_within_the_cancel_allowance_and_cleanup_runs() {
    pending_body(register_http).await;
}

#[tokio::test]
async fn peer_server_pending_body_is_destroyed_and_cleanup_runs() {
    pending_body(register_http_with_connect_info_in).await;
}

async fn pending_body(register: RegisterHttp) {
    let mut held = None;
    let served = Served::start(register, CANCEL, |router| {
        let (router, body) = held_route(router);
        held = Some(body);
        router
    })
    .await;
    let mut held = held.unwrap();
    let mut client = served.request(GET).await;
    let mut bytes = read_first_frame(&mut client).await;

    let report = served.shut_down().await;
    // The body was destroyed by its own connection task before the wrapper
    // returned: the report follows that, not a test release.
    assert!(held.dropped.try_recv().is_ok(), "body outlived the report");
    assert!(report.forced_cancellation);
    served.assert_joined_and_cleaned(&report);
    // The client sees an incomplete chunked message, never a final chunk.
    let _ = timeout(WAIT, client.read_to_end(&mut bytes)).await.unwrap();
    let text = String::from_utf8_lossy(&bytes);
    assert!(text.starts_with("HTTP/1.1 200 "), "{text}");
    assert!(!text.ends_with("0\r\n\r\n"), "{text}");
    drop(held.release);
}

/// Hyper stops reading while its buffer holds bytes of a following request
/// and then waits on the response body alone. When its first read returned
/// those bytes with the head and every write was ready, the connection never
/// waited in transport I/O, yet the close must still reach it.
#[tokio::test]
async fn pending_body_after_pipelined_bytes_is_closed_and_cleanup_runs() {
    let mut held = None;
    let served = Served::start(register_http, CANCEL, |router| {
        let (router, body) = held_route(router);
        held = Some(body);
        router
    })
    .await;
    let mut held = held.unwrap();
    // Written before this current-thread runtime can accept the connection,
    // so Hyper's first read returns the head and the following bytes at once.
    let mut client = std::net::TcpStream::connect(served.address).unwrap();
    std::io::Write::write_all(
        &mut client,
        b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\nGET /next HTTP/1.1\r\n",
    )
    .unwrap();
    client.set_nonblocking(true).unwrap();
    let mut client = TcpStream::from_std(client).unwrap();
    let mut bytes = read_first_frame(&mut client).await;

    let report = served.shut_down().await;
    assert!(held.dropped.try_recv().is_ok(), "body outlived the report");
    assert!(report.forced_cancellation);
    served.assert_joined_and_cleaned(&report);
    let _ = timeout(WAIT, client.read_to_end(&mut bytes)).await.unwrap();
    let text = String::from_utf8_lossy(&bytes);
    assert!(text.starts_with("HTTP/1.1 200 "), "{text}");
    assert!(!text.ends_with("0\r\n\r\n"), "{text}");
    drop(held.release);
}

/// A settled native component that reports success.
struct Settled;

impl ManagedSettlement for Settled {
    fn is_success(&self) -> bool {
        true
    }

    fn allows_dependency_cleanup(&self) -> bool {
        true
    }
}

/// A native component that, while stopping, holds its worker past forced
/// cancellation and then reveals a stop time `earlier` than the one it was
/// given, moving the process abort deadline earlier.
fn late_earlier_stop(supervisor: &mut Supervisor, held: Duration, earlier: Duration) {
    let context = OperationOwner::new(WAIT).unwrap().into_context();
    supervisor
        .register_managed("native", context, move |_| {
            let (stopped, settled) = oneshot::channel();
            let mut stopped = Some(stopped);
            Ok(ManagedComponent::new(
                async { Ok::<(), BoxError>(()) },
                std::future::pending(),
                move |started| {
                    if let Some(stopped) = stopped.take() {
                        std::thread::sleep(held);
                        let _ = stopped.send(());
                        return started.checked_sub(earlier).unwrap();
                    }
                    started
                },
                async move {
                    let _ = settled.await;
                    Settled
                },
            ))
        })
        .unwrap();
}

/// The close instant follows an abort deadline that moved earlier after
/// forced cancellation was observed. With a one-second cancel allowance the
/// close was due 550 ms after drain and the abort 1050 ms. The component
/// reveals, 100 ms after drain, a stop 600 ms earlier: the abort is then due
/// at 450 ms, before the original close, and the recomputed close at 250 ms.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn close_follows_an_abort_deadline_tightened_after_forced_cancellation() {
    let mut held = None;
    let served = Served::start_with(
        register_http,
        Duration::from_secs(1),
        |router| {
            let (router, body) = held_route(router);
            held = Some(body);
            router
        },
        |supervisor| {
            late_earlier_stop(
                supervisor,
                Duration::from_millis(100),
                Duration::from_millis(600),
            );
        },
    )
    .await;
    let held = held.unwrap();
    let mut client = served.request(GET).await;
    read_first_frame(&mut client).await;

    let report = served.shut_down().await;
    assert!(report.forced_cancellation);
    served.assert_joined_and_cleaned(&report);
    drop(held.release);
}

#[tokio::test]
async fn stalled_reader_of_a_fixed_body_is_closed_and_cleanup_runs() {
    // Larger than both loopback socket buffers, so the write stays pending.
    let payload = Bytes::from(vec![0_u8; 64 << 20]);
    let served = Served::start(register_http, CANCEL, |router| {
        router.route("/", get(move || async move { payload.clone() }))
    })
    .await;
    let mut client = served.request(GET).await;
    let mut head = [0_u8; 13];
    timeout(WAIT, client.read_exact(&mut head))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&head, b"HTTP/1.1 200 ");
    // The handler has finished; the client now stops reading for good.
    tokio::time::sleep(Duration::from_millis(100)).await;

    let report = served.shut_down().await;
    assert!(report.forced_cancellation);
    served.assert_joined_and_cleaned(&report);
    drop(client);
}

#[tokio::test]
async fn forced_cancellation_still_delivers_the_interruption_response_before_closing() {
    let (entered_tx, entered) = oneshot::channel();
    let entered_tx = Arc::new(Mutex::new(Some(entered_tx)));
    let served = Served::start(register_http, CANCEL, |router| {
        router.route(
            "/",
            get(move || async move {
                entered_tx.lock().unwrap().take().unwrap().send(()).unwrap();
                std::future::pending::<()>().await;
            }),
        )
    })
    .await;
    let mut client = served.request(GET).await;
    timeout(WAIT, entered).await.unwrap().unwrap();

    let report = served.shut_down().await;
    assert!(report.forced_cancellation);
    served.assert_joined_and_cleaned(&report);
    // The cancelled handler's response was written whole before the gate closed.
    let mut answered = String::new();
    timeout(WAIT, client.read_to_string(&mut answered))
        .await
        .unwrap()
        .unwrap();
    assert!(answered.starts_with("HTTP/1.1 503 "), "{answered}");
    assert!(answered.contains("operation_cancelled"), "{answered}");
}

#[tokio::test]
async fn partial_request_head_does_not_force_an_abort() {
    let served = Served::start(register_http, CANCEL, |router| {
        router.route("/", get(|| async {}))
    })
    .await;
    let client = served.request(b"GET / HTTP/1.1\r\nHost: local").await;
    // No client-visible event marks the server reading a partial head; the
    // report below proves it did.
    tokio::time::sleep(Duration::from_millis(50)).await;

    let report = served.shut_down().await;
    // Graceful shutdown ends a fresh connection at once only while Hyper has
    // read nothing from it. Outlasting drain shows the head was buffered, so
    // only the gate could end the connection before the abort.
    assert!(report.forced_cancellation, "{report:?}");
    served.assert_joined_and_cleaned(&report);
    drop(client);
}

/// A zero cancel allowance leaves no interval between cancellation and abort,
/// so the earlier contract remains: the wrapper is aborted, the body stays
/// owned by its detached connection task and dependent cleanup is skipped.
#[tokio::test]
async fn zero_cancel_allowance_still_aborts_the_wrapper_and_skips_cleanup() {
    let mut held = None;
    let served = Served::start(register_http, Duration::ZERO, |router| {
        let (router, body) = held_route(router);
        held = Some(body);
        router
    })
    .await;
    let mut held = held.unwrap();
    let mut client = served.request(GET).await;
    let mut bytes = read_first_frame(&mut client).await;

    let report = served.shut_down().await;
    let body_still_owned = matches!(
        held.dropped.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    );
    // Release and await the native connection body even if assertions fail.
    let _ = held.release.send(());
    timeout(WAIT, client.read_to_end(&mut bytes))
        .await
        .unwrap()
        .unwrap();
    timeout(WAIT, held.dropped).await.unwrap().unwrap();
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
    assert!(!served.cleaned.load(Ordering::SeqCst));
}

/// One frame, then, once started, a poll that blocks its connection task
/// until released or for at most `WAIT`: a task that never yields.
struct BlockingBody {
    first: bool,
    start: oneshot::Receiver<()>,
    blocked: Option<oneshot::Sender<()>>,
    release: std::sync::mpsc::Receiver<()>,
    returned: Arc<AtomicBool>,
}

impl http_body::Body for BlockingBody {
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<http_body::Frame<Bytes>, Infallible>>> {
        if std::mem::take(&mut self.first) {
            return Poll::Ready(Some(Ok(http_body::Frame::data(Bytes::from_static(
                b"first",
            )))));
        }
        // Pending first lets Hyper flush the first frame to the peer.
        let _ = std::task::ready!(Pin::new(&mut self.start).poll(cx));
        if let Some(blocked) = self.blocked.take() {
            let _ = blocked.send(());
        }
        let release = &self.release;
        let _ = tokio::task::block_in_place(|| release.recv_timeout(WAIT));
        self.returned.store(true, Ordering::SeqCst);
        Poll::Ready(None)
    }
}

struct Blocking {
    start: oneshot::Sender<()>,
    blocked: oneshot::Receiver<()>,
    release: std::sync::mpsc::Sender<()>,
    returned: Arc<AtomicBool>,
}

fn blocking_route(router: Router) -> (Router, Blocking) {
    let (start, started) = oneshot::channel();
    let (blocked_tx, blocked) = oneshot::channel();
    let (release, released) = std::sync::mpsc::channel();
    let returned = Arc::new(AtomicBool::new(false));
    let body = Arc::new(Mutex::new(Some(BlockingBody {
        first: true,
        start: started,
        blocked: Some(blocked_tx),
        release: released,
        returned: returned.clone(),
    })));
    let router = router.route(
        "/",
        get(move || async move { Body::new(body.lock().unwrap().take().unwrap()) }),
    );
    let blocking = Blocking {
        start,
        blocked,
        release,
        returned,
    };
    (router, blocking)
}

/// A connection task that never yields cannot observe the closed gate. It
/// outlives the close and the abort deadline, so the wrapper is aborted and
/// dependent cleanup skipped: closing the gate is not taken as proof that the
/// server's connections ended.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn connection_that_never_yields_outlives_the_close_and_the_wrapper_is_aborted() {
    let mut blocking = None;
    let served = Served::start(register_http, CANCEL, |router| {
        let (router, body) = blocking_route(router);
        blocking = Some(body);
        router
    })
    .await;
    let blocking = blocking.unwrap();
    let mut client = served.request(GET).await;
    read_first_frame(&mut client).await;
    blocking.start.send(()).unwrap();
    timeout(WAIT, blocking.blocked).await.unwrap().unwrap();

    let report = served.shut_down().await;
    let still_blocked = !blocking.returned.load(Ordering::SeqCst);
    // Release the connection task even if assertions fail.
    let _ = blocking.release.send(());
    timeout(WAIT, async {
        while !blocking.returned.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert!(
        still_blocked,
        "the connection task returned before the report"
    );
    assert!(!report.is_success());
    assert!(report.forced_cancellation);
    assert_eq!(report.abort_requested, ["http"]);
    assert!(report.all_direct_tasks_joined());
    assert_eq!(report.cleanup.skipped.len(), 1);
    assert_eq!(report.cleanup.skipped[0].name, "dependency");
    assert_eq!(report.cleanup.skipped[0].reason, SkipReason::UnsafeTaskExit);
    assert!(!served.cleaned.load(Ordering::SeqCst));
    drop(client);
}

mod slow_destructor;
