//! Owned bodies over real sockets, through the registered server: the body's
//! own ending against the server's forced connection close, and a peer that
//! goes away.

use super::support::{assert_finished, budget, finished};
use crate::capture::Capture;
use axum::{
    body::{Body, Bytes},
    http::{HeaderMap, HeaderName, HeaderValue, header},
    routing::get as route,
};
use batter_axum::{
    AdmittedRequest, GuardedRouter, HttpBoundary, RequestPolicy, ResponseConstructionBudget,
};
use batter_core::{
    cleanup::CleanupBudget,
    lifecycle::{RunningSupervisor, ShutdownBudget, ShutdownHandle, Supervisor},
};
use futures_util::stream::{self, StreamExt};
use http_body::Frame;
use std::{
    convert::Infallible,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    task::{Context, Poll},
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    time::timeout,
};

const WAIT: Duration = Duration::from_secs(10);
const DRAIN: Duration = Duration::from_millis(50);
/// The server closes connection I/O halfway through this, one second after
/// forced cancellation; a body that ends itself is well inside that.
const CANCEL: Duration = Duration::from_secs(2);
const GET: &[u8] = b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n";

struct Served {
    handle: ShutdownHandle,
    address: std::net::SocketAddr,
    cleaned: Arc<AtomicBool>,
    running: RunningSupervisor,
}

async fn serve(guarded: GuardedRouter) -> Served {
    let cleanup = CleanupBudget::new(WAIT, WAIT, WAIT).unwrap();
    let mut supervisor =
        Supervisor::new(ShutdownBudget::new(DRAIN, CANCEL, WAIT, cleanup).unwrap());
    let handle = supervisor.handle();
    let cleaned = Arc::new(AtomicBool::new(false));
    let finalized = cleaned.clone();
    supervisor
        .on_cleanup("dependency", move || async move {
            finalized.store(true, Ordering::SeqCst);
            Ok(())
        })
        .unwrap();
    let policy = RequestPolicy::new(
        handle.operation_admission(),
        ResponseConstructionBudget::new(WAIT).unwrap(),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    HttpBoundary::new(policy)
        .assemble(guarded)
        .await
        .unwrap()
        .register_in(&mut supervisor, "http", listener)
        .unwrap();
    let running = supervisor.start();
    timeout(WAIT, handle.status().wait_ready())
        .await
        .unwrap()
        .unwrap();
    Served {
        handle,
        address,
        cleaned,
        running,
    }
}

impl Served {
    async fn request(&self) -> TcpStream {
        let mut client = TcpStream::connect(self.address).await.unwrap();
        client.write_all(GET).await.unwrap();
        client
    }

    /// Shut down, requiring a joined server and a run finalizer.
    async fn shut_down(&self) -> Duration {
        let started = Instant::now();
        self.handle.request();
        let report = timeout(WAIT, self.running.wait()).await.unwrap().unwrap();
        assert!(report.is_success(), "{report:?}");
        assert!(report.abort_requested.is_empty(), "{report:?}");
        assert!(report.cleanup.skipped.is_empty(), "{report:?}");
        assert!(self.cleaned.load(Ordering::SeqCst));
        started.elapsed()
    }
}

async fn read_until(client: &mut TcpStream, marker: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    timeout(WAIT, async {
        while !bytes.windows(marker.len()).any(|window| window == marker) {
            assert_ne!(client.read_buf(&mut bytes).await.unwrap(), 0);
        }
    })
    .await
    .unwrap();
    bytes
}

/// One frame, then pending forever.
fn stuck() -> Body {
    Body::from_stream(stream::unfold(true, |first| async move {
        if !first {
            std::future::pending::<()>().await;
        }
        Some((Ok::<_, Infallible>(Bytes::from_static(b"first;")), false))
    }))
}

async fn wait_for_finished(capture: &Capture) {
    timeout(WAIT, async {
        while finished(capture).is_empty() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
}

/// A pending owned body is woken by forced cancellation and ends itself, so
/// the connection closes without waiting for the server's forced close.
#[test]
fn a_pending_owned_body_ends_itself_at_forced_cancellation_before_the_forced_close() {
    let capture = Capture::new();
    let (elapsed, text) = capture.block_on(async {
        let served = serve(GuardedRouter::new().route(
            "/",
            route(|admitted: AdmittedRequest| async move {
                admitted.into_streaming_response(budget(600, 300), |_scope| stuck())
            }),
        ))
        .await;
        let mut client = served.request().await;
        let mut bytes = read_until(&mut client, b"first;").await;
        let elapsed = served.shut_down().await;
        let _ = timeout(WAIT, client.read_to_end(&mut bytes)).await.unwrap();
        (elapsed, String::from_utf8_lossy(&bytes).into_owned())
    });
    assert!(
        elapsed < DRAIN + CANCEL / 2,
        "the body waited for the forced close: {elapsed:?}"
    );
    // Truncated: the chunked message never received its final chunk.
    assert!(text.starts_with("HTTP/1.1 200 "), "{text}");
    assert!(!text.ends_with("0\r\n\r\n"), "{text}");
    assert_finished(&capture, "WARN", "cancelled");
}

/// A peer that stops reading leaves the body unpolled, so only the server's
/// forced close ends it. The body is then destroyed after its scope was
/// cancelled and reports that, not a peer that went away.
#[test]
fn a_stalled_reader_is_ended_by_the_forced_close_and_observed_as_cancelled() {
    let capture = Capture::new();
    let produced = Arc::new(AtomicUsize::new(0));
    let counted = produced.clone();
    let elapsed = capture.block_on(async {
        let served = serve(GuardedRouter::new().route(
            "/",
            route(move |admitted: AdmittedRequest| async move {
                admitted.into_streaming_response(budget(600, 300), move |_scope| {
                    // Always ready: far more than both socket buffers hold.
                    Body::from_stream(stream::repeat_with(move || {
                        counted.fetch_add(1, Ordering::SeqCst);
                        Ok::<_, Infallible>(Bytes::from(vec![0_u8; 1 << 20]))
                    }))
                })
            }),
        ))
        .await;
        let mut client = served.request().await;
        let mut head = [0_u8; 13];
        timeout(WAIT, client.read_exact(&mut head))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(&head, b"HTTP/1.1 200 ");
        // Shut down only once the transport is write-blocked. This runtime has
        // one thread, so a server task that can still write stays ready and
        // keeps producing while this task waits; a count unchanged across an
        // idle wait means both socket buffers are full and polling stopped.
        timeout(WAIT, async {
            let mut seen = 0;
            loop {
                tokio::time::sleep(Duration::from_millis(50)).await;
                let now = produced.load(Ordering::SeqCst);
                if now > 0 && now == seen {
                    break;
                }
                seen = now;
            }
        })
        .await
        .unwrap();
        let elapsed = served.shut_down().await;
        drop(client);
        elapsed
    });
    assert!(
        elapsed >= DRAIN + CANCEL / 2,
        "ended before the forced close: {elapsed:?}"
    );
    assert_finished(&capture, "WARN", "cancelled");
}

#[test]
fn a_peer_that_goes_away_destroys_the_body_which_is_observed_as_dropped() {
    let capture = Capture::new();
    capture.block_on(async {
        let served = serve(GuardedRouter::new().route(
            "/",
            route(|admitted: AdmittedRequest| async move {
                admitted.into_streaming_response(budget(600, 300), |_scope| stuck())
            }),
        ))
        .await;
        let mut client = served.request().await;
        read_until(&mut client, b"first;").await;
        client.shutdown().await.unwrap();
        drop(client);
        // Observed while the process is still serving, before any shutdown.
        wait_for_finished(&capture).await;
        assert_finished(&capture, "INFO", "dropped");
        served.shut_down().await;
    });
    assert_eq!(finished(&capture).len(), 1);
}

/// Hyper learns a sized body's end from `is_end_stream` and never polls for
/// it, so the owner must recognize the finished source when it is destroyed.
#[test]
fn a_sized_owned_body_keeps_its_length_and_is_observed_as_completed() {
    let capture = Capture::new();
    let text = capture.block_on(async {
        let served = serve(GuardedRouter::new().route(
            "/",
            route(|admitted: AdmittedRequest| async move {
                admitted.into_streaming_response(budget(600, 300), |_scope| "hello")
            }),
        ))
        .await;
        let mut client = served.request().await;
        let mut text = String::new();
        timeout(WAIT, client.read_to_string(&mut text))
            .await
            .unwrap()
            .unwrap();
        wait_for_finished(&capture).await;
        served.shut_down().await;
        text
    });
    assert!(text.starts_with("HTTP/1.1 200 "), "{text}");
    assert!(text.contains("content-length: 5\r\n"), "{text}");
    assert!(text.ends_with("\r\n\r\nhello"), "{text}");
    let event = assert_finished(&capture, "INFO", "completed");
    assert!(event.contains("bytes=5"), "{event}");
}

/// Hyper writes an advertised length and then stops polling, without asking
/// the stream for its end, so a body that produced that length completed even
/// though its source never ends.
#[test]
fn a_streamed_body_that_produced_its_declared_length_is_observed_as_completed() {
    let capture = Capture::new();
    let text = capture.block_on(async {
        let served = serve(GuardedRouter::new().route(
            "/",
            route(|admitted: AdmittedRequest| async move {
                admitted.into_streaming_response(budget(600, 300), |_scope| {
                    let chunks = [Bytes::from_static(b"hello"), Bytes::from_static(b"world")];
                    let source =
                        stream::iter(chunks.map(Ok::<_, Infallible>)).chain(stream::pending());
                    ([(header::CONTENT_LENGTH, "10")], Body::from_stream(source))
                })
            }),
        ))
        .await;
        let mut client = served.request().await;
        let mut text = String::new();
        timeout(WAIT, client.read_to_string(&mut text))
            .await
            .unwrap()
            .unwrap();
        wait_for_finished(&capture).await;
        served.shut_down().await;
        text
    });
    assert!(text.contains("content-length: 10\r\n"), "{text}");
    assert!(text.ends_with("\r\n\r\nhelloworld"), "{text}");
    let event = assert_finished(&capture, "INFO", "completed");
    assert!(event.contains("bytes=10"), "{event}");
    assert_eq!(finished(&capture).len(), 1);
}

/// Data, then trailers, then pending: the source never yields its end.
struct Trailed {
    frames: u8,
}

impl http_body::Body for Trailed {
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Infallible>>> {
        self.frames += 1;
        match self.frames {
            1 => Poll::Ready(Some(Ok(Frame::data(Bytes::from_static(b"hello"))))),
            2 => Poll::Ready(Some(Ok(Frame::trailers(HeaderMap::from_iter([(
                HeaderName::from_static("x-digest"),
                HeaderValue::from_static("done"),
            )]))))),
            _ => Poll::Pending,
        }
    }
}

/// Trailers are a body's last frame: Hyper writes them and destroys the body
/// without polling again, so the owner records a completed body.
#[test]
fn a_body_that_ended_with_trailers_is_observed_as_completed() {
    let capture = Capture::new();
    let text = capture.block_on(async {
        let served = serve(GuardedRouter::new().route(
            "/",
            route(|admitted: AdmittedRequest| async move {
                admitted.into_streaming_response(budget(600, 300), |_scope| {
                    // Hyper writes only the trailer fields a response declares.
                    (
                        [(header::TRAILER, "x-digest")],
                        Body::new(Trailed { frames: 0 }),
                    )
                })
            }),
        ))
        .await;
        let mut client = TcpStream::connect(served.address).await.unwrap();
        client
            .write_all(
                b"GET / HTTP/1.1\r\nHost: localhost\r\nTE: trailers\r\nConnection: close\r\n\r\n",
            )
            .await
            .unwrap();
        let mut text = String::new();
        timeout(WAIT, client.read_to_string(&mut text))
            .await
            .unwrap()
            .unwrap();
        wait_for_finished(&capture).await;
        served.shut_down().await;
        text
    });
    assert!(text.contains("transfer-encoding: chunked\r\n"), "{text}");
    assert!(text.ends_with("0\r\nx-digest: done\r\n\r\n"), "{text}");
    let event = assert_finished(&capture, "INFO", "completed");
    assert!(event.contains("bytes=5"), "{event}");
    assert_eq!(finished(&capture).len(), 1);
}
