//! Deterministic stalls at the handoff and socket writer, using the production
//! connection driver and gate. The peer never drives the client connection.

use super::*;
use crate::{Exchange, ResponseLimit, StartRefusal, send};
use bytes::Bytes;
use http_body_util::Full;
use hyper::client::conn::http1;
use hyper_util::rt::TokioIo;
use std::{
    future::{Future, poll_fn},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::io::{AsyncReadExt, DuplexStream};

struct Writer {
    stream: DuplexStream,
    writable: Arc<AtomicBool>,
    written: Arc<AtomicUsize>,
}

impl AsyncRead for Writer {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_read(cx, buf)
    }
}

impl AsyncWrite for Writer {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        if !self.writable.load(Ordering::SeqCst) {
            return Poll::Pending;
        }
        let result = Pin::new(&mut self.stream).poll_write(cx, bytes);
        if let Poll::Ready(Ok(count)) = result {
            self.written.fetch_add(count, Ordering::SeqCst);
        }
        result
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_shutdown(cx)
    }
}

async fn poll_pending(future: Pin<&mut impl Future<Output = Exchange>>) {
    let mut future = future;
    poll_fn(|cx| {
        assert!(
            future.as_mut().poll(cx).is_pending(),
            "exchange completed before the stall"
        );
        Poll::Ready(())
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn expired_or_cancelled_handoff_never_submits_a_request() {
    for cancel in [false, true] {
        let gate = Gate {
            deadline: Instant::now() + Duration::from_secs(1),
            is_cancelled: || cancel,
        };
        let (stream, mut peer) = tokio::io::duplex(64);
        let connected = http1::handshake(TokioIo::new(Guarded {
            inner: stream,
            gate: &gate,
        }))
        .await
        .unwrap();
        if !cancel {
            tokio::time::advance(Duration::from_secs(1)).await;
        }
        let timer = tokio::time::sleep_until(gate.deadline);
        let cancelled = std::future::pending();
        tokio::pin!(timer, cancelled);
        let request = http::Request::builder()
            .uri("/effect")
            .header("host", "localhost")
            .body(Full::new(Bytes::new()))
            .unwrap();
        let outcome = send::send_on(
            connected,
            request,
            &gate,
            ResponseLimit::new(1024).unwrap(),
            timer,
            cancelled,
        )
        .await;
        assert!(
            matches!(outcome, Exchange::NotStarted(ref refusal) if refusal.reason() == if cancel { StartRefusal::Cancelled } else { StartRefusal::DeadlineTooClose }),
            "{outcome:?}"
        );
        let mut received = Vec::new();
        peer.read_to_end(&mut received).await.unwrap();
        assert!(
            received.is_empty(),
            "handoff wrote a request after stopping"
        );
    }
}

#[tokio::test(start_paused = true)]
async fn stalled_exchange_has_no_background_writer_and_never_resumes_writing_after_stop() {
    // Exercise a queued request (zero written) and a partially written body,
    // with both interruption reasons. These are the two late-write windows.
    for partial in [false, true] {
        for cancel in [false, true] {
            let cancelled_flag = AtomicBool::new(false);
            let gate = Gate {
                deadline: Instant::now() + Duration::from_secs(1),
                is_cancelled: || cancelled_flag.load(Ordering::SeqCst),
            };
            let writable = Arc::new(AtomicBool::new(partial));
            let written = Arc::new(AtomicUsize::new(0));
            let (stream, mut peer) = tokio::io::duplex(64);
            let writer = Writer {
                stream,
                writable: writable.clone(),
                written: written.clone(),
            };
            let connected = http1::handshake(TokioIo::new(Guarded {
                inner: writer,
                gate: &gate,
            }))
            .await
            .unwrap();
            let timer = tokio::time::sleep_until(gate.deadline);
            let cancelled = poll_fn(|_| {
                if cancelled_flag.load(Ordering::SeqCst) {
                    Poll::Ready(())
                } else {
                    Poll::Pending
                }
            });
            tokio::pin!(timer, cancelled);
            let request = http::Request::builder()
                .method("POST")
                .uri("/effect")
                .header("host", "localhost")
                .body(Full::new(Bytes::from(vec![b'x'; 8192])))
                .unwrap();
            let exchange = send::send_on(
                connected,
                request,
                &gate,
                ResponseLimit::new(1024).unwrap(),
                timer,
                cancelled,
            );
            tokio::pin!(exchange);
            poll_pending(exchange.as_mut()).await;
            let before = written.load(Ordering::SeqCst);
            assert_eq!(before > 0, partial, "wrong stall phase");
            writable.store(true, Ordering::SeqCst);
            let mut received = vec![0; before];
            peer.read_exact(&mut received).await.unwrap();
            // Make the writer runnable without polling its exchange. A spawned
            // connection task would now send the queued request/body.
            tokio::task::yield_now().await;
            assert_eq!(
                written.load(Ordering::SeqCst),
                before,
                "a background task wrote request bytes"
            );
            if cancel {
                cancelled_flag.store(true, Ordering::SeqCst);
            } else {
                tokio::time::advance(Duration::from_secs(1)).await;
            }
            let outcome = exchange.await;
            assert_eq!(
                outcome.label(),
                if cancel {
                    "unknown.cancelled"
                } else {
                    "unknown.deadline"
                }
            );
            assert_eq!(
                written.load(Ordering::SeqCst),
                before,
                "resuming the exchange wrote late bytes"
            );
        }
    }
}
