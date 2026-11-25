//! What the owner does not know. An owned body is judged by the length its
//! builder's response declared and by what its source did, never by how the
//! transport framed or wrote the message, so these outcomes describe the
//! body, not what the peer received.

use super::*;
use axum::{http::StatusCode, response::IntoResponse};
use std::sync::Mutex;
use tokio::sync::oneshot;

/// Serve `source` and declare `length` only after the hand-over, on the tuple
/// the handler returns, then return what the peer read before the connection
/// ended.
fn served_declared_afterwards(
    capture: &Capture,
    length: &'static str,
    source: fn() -> Body,
) -> String {
    capture.block_on(async {
        let served = serve(GuardedRouter::new().route(
            "/",
            route(move |admitted: AdmittedRequest| async move {
                let response =
                    admitted.into_streaming_response(budget(600, 300), move |_scope| source());
                ([(header::CONTENT_LENGTH, length)], response)
            }),
        ))
        .await;
        let mut client = served.request().await;
        let mut bytes = Vec::new();
        // A short message is aborted, so the read may end in a reset.
        let _ = timeout(WAIT, client.read_to_end(&mut bytes)).await.unwrap();
        wait_for_finished(capture).await;
        served.shut_down().await;
        String::from_utf8_lossy(&bytes).into_owned()
    })
}

/// `hello`, then the source's end once `released` resolves.
fn ends_when_released(released: oneshot::Receiver<()>) -> Body {
    let first = stream::iter([Ok::<_, Infallible>(Bytes::from_static(b"hello"))]);
    let end = stream::once(released).filter_map(|_| std::future::ready(None));
    Body::from_stream(first.chain(end))
}

/// A length added after the hand-over is not the builder's, so the outcome
/// ignores it. Here the handler's tuple declares ten bytes over a source that
/// yields five and ends only once the peer holds the 200 head, its ten-byte
/// declaration and those five bytes. Hyper, framing by the later length,
/// then ends the connection short of it, and the source, which ran to its
/// end, is `completed` all the same.
#[test]
fn a_source_ending_short_of_a_later_length_is_completed_beside_an_aborted_message() {
    let capture = Capture::new();
    let text = capture.block_on(async {
        let (release, released) = oneshot::channel();
        let released = Arc::new(Mutex::new(Some(released)));
        let served = serve(GuardedRouter::new().route(
            "/",
            route(move |admitted: AdmittedRequest| {
                let released = released.lock().unwrap().take().unwrap();
                async move {
                    let response = admitted
                        .into_streaming_response(budget(600, 300), move |_scope| {
                            ends_when_released(released)
                        });
                    ([(header::CONTENT_LENGTH, "10")], response)
                }
            }),
        ))
        .await;
        let mut client = served.request().await;
        let mut bytes = Vec::new();
        timeout(WAIT, async {
            while !bytes.ends_with(b"\r\n\r\nhello") {
                assert_ne!(client.read_buf(&mut bytes).await.unwrap(), 0);
            }
        })
        .await
        .unwrap();
        assert!(finished(&capture).is_empty(), "{}", capture.text());
        release.send(()).unwrap();
        // Hyper aborts the message, so the read may end in a reset.
        let _ = timeout(WAIT, client.read_to_end(&mut bytes)).await.unwrap();
        wait_for_finished(&capture).await;
        served.shut_down().await;
        String::from_utf8_lossy(&bytes).into_owned()
    });
    assert!(text.starts_with("HTTP/1.1 200 "), "{text}");
    assert!(text.contains("\r\ncontent-length: 10\r\n"), "{text}");
    assert!(text.ends_with("\r\n\r\nhello"), "{text}");
    let event = assert_finished(&capture, "INFO", "completed");
    assert!(event.contains("bytes=5"), "{event}");
}

/// The other way round: the handler's tuple declares five bytes over a
/// source that yields five and then stays pending. Over HTTP/1, Hyper stops
/// polling once the later length is written and destroys the body, which had
/// declared nothing itself and was not at its end, so it is `dropped`
/// although the peer received a complete message. Declare the length inside
/// the builder, as `sockets::a_streamed_body_that_produced_its_declared_length_is_observed_as_completed`
/// does, for the outcome to follow it.
#[test]
fn a_source_pending_at_a_later_length_is_dropped_beside_a_complete_message() {
    let capture = Capture::new();
    let text = served_declared_afterwards(&capture, "5", || {
        let source = stream::iter([Ok::<_, Infallible>(Bytes::from_static(b"hello"))]);
        Body::from_stream(source.chain(stream::pending()))
    });
    assert!(text.starts_with("HTTP/1.1 200 "), "{text}");
    assert!(text.contains("content-length: 5\r\n"), "{text}");
    assert!(text.ends_with("\r\n\r\nhello"), "{text}");
    let event = assert_finished(&capture, "INFO", "dropped");
    assert!(event.contains("bytes=5"), "{event}");
}

/// Five bytes and then pending, never its end. Its size is unknown until its
/// end has been asked once and exact afterwards, as a body fed by a producer
/// that finishes after the hand-over answers. The hand-over asks only its
/// size; Hyper asks its end, then its size, before writing the head.
struct SizedOnlyLater {
    end_asked: AtomicBool,
    sent: bool,
}

impl http_body::Body for SizedOnlyLater {
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Infallible>>> {
        if std::mem::replace(&mut self.sent, true) {
            return Poll::Pending;
        }
        Poll::Ready(Some(Ok(Frame::data(Bytes::from_static(b"hello")))))
    }

    fn is_end_stream(&self) -> bool {
        self.end_asked.store(true, Ordering::SeqCst);
        false
    }

    fn size_hint(&self) -> SizeHint {
        // The bytes still to come, as `http-body` defines it: none once the
        // five were yielded, although the source never reports its end.
        match (self.end_asked.load(Ordering::SeqCst), self.sent) {
            (false, _) => SizeHint::default(),
            (true, false) => SizeHint::with_exact(5),
            (true, true) => SizeHint::with_exact(0),
        }
    }
}

/// A size the body reports only after the hand-over was not declared either:
/// Hyper frames the message by it and stops polling after five bytes, and the
/// body, whose source never reached its end, is `dropped` with those bytes
/// counted. `dropped` therefore does not say the peer's message was cut.
#[test]
fn a_size_reported_only_after_the_hand_over_does_not_decide_the_outcome() {
    let capture = Capture::new();
    let text = capture.block_on(async {
        let served = serve(GuardedRouter::new().route(
            "/",
            route(|admitted: AdmittedRequest| async move {
                admitted.into_streaming_response(budget(600, 300), |_scope| {
                    Body::new(SizedOnlyLater {
                        end_asked: AtomicBool::new(false),
                        sent: false,
                    })
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
    assert!(text.contains("content-length: 5\r\n"), "{text}");
    assert!(text.ends_with("\r\n\r\nhello"), "{text}");
    let event = assert_finished(&capture, "INFO", "dropped");
    assert!(event.contains("bytes=5"), "{event}");
}

/// Polls of the sources of [`a_head_the_transport_refuses_is_not_detected`],
/// the only test that reads it.
static REFUSED_POLLS: AtomicUsize = AtomicUsize::new(0);

/// A body that counts every poll of it and answers for its end and size as
/// `inner` does.
struct Counted(Body);

impl http_body::Body for Counted {
    type Data = Bytes;
    type Error = axum::Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, axum::Error>>> {
        REFUSED_POLLS.fetch_add(1, Ordering::SeqCst);
        Pin::new(&mut self.0).poll_frame(cx)
    }

    fn is_end_stream(&self) -> bool {
        self.0.is_end_stream()
    }

    fn size_hint(&self) -> SizeHint {
        self.0.size_hint()
    }
}

/// A counted body whose source never yields.
fn pending() -> Body {
    Body::new(Counted(Body::from_stream(stream::pending::<
        Result<Bytes, Infallible>,
    >())))
}

/// A counted body already at its end.
fn empty() -> Body {
    Body::new(Counted(Body::empty()))
}

/// A final 103 answer, which declares no length whatever its headers say.
fn early_hints(body: Body) -> Response {
    Response::builder()
        .status(StatusCode::EARLY_HINTS)
        .body(body)
        .unwrap()
}

/// A 200 answer declaring `length` beside `Transfer-Encoding`.
fn length_beside_chunked(length: &'static str, body: Body) -> Response {
    Response::builder()
        .header(header::CONTENT_LENGTH, length)
        .header(header::TRANSFER_ENCODING, "chunked")
        .body(body)
        .unwrap()
}

/// Hyper writes no final 1xx answer but 101, which it answers with a 500 in
/// its place, and no answer whose `Content-Length` precedes a
/// `Transfer-Encoding` over a body not yet at its end, where it closes the
/// connection with nothing written. It polls the body of neither, which the
/// sources count. The owner cannot see the refusal, and its outcome does not
/// detect it: the unpolled body is judged like any other destroyed unpolled,
/// by what its builder's response declared and what its source reports. A pending source with no
/// length declared is `dropped`, like one whose peer went away; a source
/// already at its end is `completed`; a pending source that declared zero
/// bytes is `completed` too, having yielded all it declared; and a source at
/// its end short of the five bytes its builder declared, under a 103 status
/// the handler's tuple set afterwards, is `failed`. The peer received none of
/// them.
#[test]
fn a_head_the_transport_refuses_is_not_detected() {
    /// What the peer reads in place of the refused answer.
    const REPLACED: &str = "HTTP/1.1 500 ";
    const NOTHING: &str = "";
    fn owned(admitted: AdmittedRequest, built: fn() -> Response) -> Response {
        admitted.into_streaming_response(budget(600, 300), move |_scope| built())
    }
    /// A handler, what the peer reads, and the body event's level and outcome.
    type Case = (
        fn(AdmittedRequest) -> Response,
        &'static str,
        &'static str,
        &'static str,
    );
    let cases: [Case; 4] = [
        (
            |admitted| owned(admitted, || early_hints(pending())),
            REPLACED,
            "INFO",
            "dropped",
        ),
        (
            |admitted| owned(admitted, || early_hints(empty())),
            REPLACED,
            "INFO",
            "completed",
        ),
        (
            |admitted| owned(admitted, || length_beside_chunked("0", pending())),
            NOTHING,
            "INFO",
            "completed",
        ),
        (
            |admitted| {
                let declared = || ([(header::CONTENT_LENGTH, "5")], empty()).into_response();
                (StatusCode::EARLY_HINTS, owned(admitted, declared)).into_response()
            },
            REPLACED,
            "WARN",
            "failed",
        ),
    ];
    for (case, (handler, answered, level, outcome)) in cases.into_iter().enumerate() {
        REFUSED_POLLS.store(0, Ordering::SeqCst);
        let capture = Capture::new();
        let text = capture.block_on(async {
            let served = serve(GuardedRouter::new().route(
                "/",
                route(move |admitted: AdmittedRequest| async move { handler(admitted) }),
            ))
            .await;
            let mut client = served.request().await;
            let mut bytes = Vec::new();
            let _ = timeout(WAIT, client.read_to_end(&mut bytes)).await.unwrap();
            wait_for_finished(&capture).await;
            served.shut_down().await;
            String::from_utf8_lossy(&bytes).into_owned()
        });
        if answered.is_empty() {
            assert!(text.is_empty(), "case {case}: {text}");
        } else {
            assert!(text.starts_with(answered), "case {case}: {text}");
        }
        let event = assert_finished(&capture, level, outcome);
        assert!(event.contains("bytes=0"), "case {case}: {event}");
        assert_eq!(REFUSED_POLLS.load(Ordering::SeqCst), 0, "case {case}");
    }
}
