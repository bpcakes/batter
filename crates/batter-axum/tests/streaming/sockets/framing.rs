//! How the registered server frames an owned body, and so how its outcome is
//! judged: lengths Hyper refuses, a version that changes the framing, a
//! length declared outside request admission, and the message that carries
//! the body, wherever its admission ran.

use super::*;

/// Claims an exact size of ten bytes, yields five and ends.
struct Overstated {
    sent: bool,
}

impl http_body::Body for Overstated {
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Infallible>>> {
        if std::mem::replace(&mut self.sent, true) {
            return Poll::Ready(None);
        }
        Poll::Ready(Some(Ok(Frame::data(Bytes::from_static(b"hello")))))
    }

    fn size_hint(&self) -> SizeHint {
        SizeHint::with_exact(10)
    }
}

/// Axum's route declares a `Content-Length` for a body of exact size even
/// beside a `Transfer-Encoding` header, after every layer of the router, and
/// Hyper refuses a message carrying both: the peer receives nothing, and the
/// owned body, never polled, is reported `failed`, not as a departed peer. So
/// through Axum's router Hyper chunk-encodes only a body of unknown size.
#[test]
fn a_transfer_encoding_beside_a_body_of_exact_size_is_refused() {
    let capture = Capture::new();
    let text = capture.block_on(async {
        let served = serve(GuardedRouter::new().route(
            "/",
            route(|admitted: AdmittedRequest| async move {
                admitted.into_streaming_response(budget(600, 300), |_scope| {
                    (
                        [(header::TRANSFER_ENCODING, "chunked")],
                        Body::new(Overstated { sent: false }),
                    )
                })
            }),
        ))
        .await;
        let mut client = served.request().await;
        let mut bytes = Vec::new();
        let _ = timeout(WAIT, client.read_to_end(&mut bytes)).await.unwrap();
        wait_for_finished(&capture).await;
        served.shut_down().await;
        String::from_utf8_lossy(&bytes).into_owned()
    });
    assert_eq!(text, "");
    let event = assert_finished(&capture, "WARN", "failed");
    assert!(event.contains("bytes=0"), "{event}");
}

/// Hyper answers in the response's own version unless the request was
/// HTTP/1.0, and an HTTP/1.0 answer drops `Transfer-Encoding`: an HTTP/1.0
/// response to an HTTP/1.1 request is framed by its `Content-Length`, so a
/// source ending short of it is `failed`, not taken for a chunked message.
#[test]
fn an_http10_response_is_framed_by_its_length_despite_transfer_encoding() {
    let capture = Capture::new();
    let text = capture.block_on(async {
        let served = serve(GuardedRouter::new().route(
            "/",
            route(|admitted: AdmittedRequest| async move {
                admitted.into_streaming_response(budget(600, 300), |_scope| {
                    let source = stream::iter([Ok::<_, Infallible>(Bytes::from_static(b"hello"))]);
                    Response::builder()
                        .version(Version::HTTP_10)
                        .header(header::TRANSFER_ENCODING, "chunked")
                        .header(header::CONTENT_LENGTH, "10")
                        .body(Body::from_stream(source))
                        .unwrap()
                })
            }),
        ))
        .await;
        let mut client = served.request().await;
        let mut bytes = Vec::new();
        // Hyper aborts the message, so the read may end in a reset.
        let _ = timeout(WAIT, client.read_to_end(&mut bytes)).await.unwrap();
        wait_for_finished(&capture).await;
        served.shut_down().await;
        String::from_utf8_lossy(&bytes).into_owned()
    });
    assert_short_and_failed(&capture, &text);
}

/// A layer between request admission and the operational wrapper that
/// declares a length in place.
fn declare_ten(outside: Router) -> Router {
    outside.layer(axum::middleware::map_response(
        |mut response: Response| async move {
            response
                .headers_mut()
                .insert(header::CONTENT_LENGTH, HeaderValue::from_static("10"));
            response
        },
    ))
}

/// The same layer rebuilding the response around its body, which keeps
/// nothing else the response carried.
fn rebuild_with_ten(outside: Router) -> Router {
    outside.layer(axum::middleware::map_response(
        |response: Response| async move {
            Response::builder()
                .header(header::CONTENT_LENGTH, "10")
                .body(response.into_body())
                .unwrap()
        },
    ))
}

/// On the registered server the length is settled from the response handed
/// to Hyper: a layer between request admission and the operational wrapper
/// that declares one frames the message, so a source ending short of it is
/// `failed`, since Hyper aborts that message. That holds whether the layer
/// edits the response or rebuilds it, and under a second, nested admission,
/// whose record is linked to the server's.
#[test]
fn a_length_declared_outside_admission_decides_the_outcome() {
    for outside in [declare_ten as fn(Router) -> Router, rebuild_with_ten] {
        for admissions in [1, 2] {
            length_declared_outside(admissions, outside);
        }
    }
}

fn length_declared_outside(admissions: usize, outside: fn(Router) -> Router) {
    let capture = Capture::new();
    let text = capture.block_on(async {
        let router = Router::new().route(
            "/",
            route(|admitted: AdmittedRequest| async move {
                admitted.into_streaming_response(budget(600, 300), |_scope| {
                    Body::from_stream(stream::iter([Ok::<_, Infallible>(Bytes::from_static(
                        b"hello",
                    ))]))
                })
            }),
        );
        let served = serve_composed(Composed::LowLevel(router, outside, admissions)).await;
        let mut client = served.request().await;
        let mut bytes = Vec::new();
        // Hyper aborts the message, so the read may end in a reset.
        let _ = timeout(WAIT, client.read_to_end(&mut bytes)).await.unwrap();
        wait_for_finished(&capture).await;
        served.shut_down().await;
        String::from_utf8_lossy(&bytes).into_owned()
    });
    // Hyper aborts the message, so the peer may receive none of it.
    assert_short_and_failed(&capture, &text);
}

/// A `Content-Length` before `Transfer-Encoding` makes Hyper refuse the head
/// even in an HTTP/1.0 answer and even for a length of zero, which a body
/// never polled would otherwise appear to have produced: it is `failed`.
#[test]
fn a_refused_zero_length_head_is_failed_not_completed() {
    let capture = Capture::new();
    let text = capture.block_on(async {
        let served = serve(GuardedRouter::new().route(
            "/",
            route(|admitted: AdmittedRequest| async move {
                admitted.into_streaming_response(budget(600, 300), |_scope| {
                    Response::builder()
                        .version(Version::HTTP_10)
                        .header(header::CONTENT_LENGTH, "0")
                        .header(header::TRANSFER_ENCODING, "chunked")
                        .body(Body::from_stream(stream::pending::<
                            Result<Bytes, Infallible>,
                        >()))
                        .unwrap()
                })
            }),
        ))
        .await;
        let mut client = served.request().await;
        let mut bytes = Vec::new();
        let _ = timeout(WAIT, client.read_to_end(&mut bytes)).await.unwrap();
        wait_for_finished(&capture).await;
        served.shut_down().await;
        String::from_utf8_lossy(&bytes).into_owned()
    });
    assert_eq!(text, "");
    let event = assert_finished(&capture, "WARN", "failed");
    assert!(event.contains("bytes=0"), "{event}");
}

/// What a final 1xx answer carries.
#[derive(Clone, Copy, Debug)]
enum Carried {
    Pending,
    Ended,
    PolledByTheHandler,
}

/// Hyper writes no final 1xx answer but 101: it sends a 500 instead and
/// never polls the body, so an owned body under such a head is `failed`, not
/// taken for a departed peer or a completed empty message, even when it had
/// already ended or its handler polled it first.
#[test]
fn a_final_informational_head_is_refused_and_failed() {
    for carried in [
        Carried::Pending,
        Carried::Ended,
        Carried::PolledByTheHandler,
    ] {
        informational_head_refused(carried);
    }
}

fn informational_head_refused(carried: Carried) {
    use axum::body::HttpBody as _;
    let capture = Capture::new();
    let text = capture.block_on(async {
        let served = serve(GuardedRouter::new().route(
            "/",
            route(move |admitted: AdmittedRequest| async move {
                let response = admitted.into_streaming_response(budget(600, 300), move |_scope| {
                    let body = match carried {
                        Carried::Ended => Body::empty(),
                        Carried::Pending | Carried::PolledByTheHandler => {
                            Body::from_stream(stream::pending::<Result<Bytes, Infallible>>())
                        }
                    };
                    Response::builder()
                        .status(axum::http::StatusCode::EARLY_HINTS)
                        .body(body)
                        .unwrap()
                });
                let (parts, mut body) = response.into_parts();
                if matches!(carried, Carried::PolledByTheHandler) {
                    std::future::poll_fn(|cx| {
                        assert!(Pin::new(&mut body).poll_frame(cx).is_pending());
                        Poll::Ready(())
                    })
                    .await;
                }
                Response::from_parts(parts, body)
            }),
        ))
        .await;
        let mut client = served.request().await;
        let mut bytes = Vec::new();
        let _ = timeout(WAIT, client.read_to_end(&mut bytes)).await.unwrap();
        wait_for_finished(&capture).await;
        served.shut_down().await;
        String::from_utf8_lossy(&bytes).into_owned()
    });
    assert!(!text.contains(" 103 "), "{carried:?}: {text}");
    let event = assert_finished(&capture, "WARN", "failed");
    assert!(event.contains("bytes=0"), "{carried:?}: {event}");
}

/// A layer outside request admission that polls the inner service's future
/// on a task of its own, inside one that declares a length.
fn spawn_then_declare_ten(outside: Router) -> Router {
    declare_ten(outside.layer(axum::middleware::from_fn(
        |request: axum::extract::Request, next: axum::middleware::Next| async move {
            tokio::spawn(next.run(request))
                .await
                .expect("the inner service returns")
        },
    )))
}

/// Admission that a layer polls on another task is judged like any other: the
/// transport writing the response reaches the body through every layer, so a
/// length declared outside admission frames it, and a source falling short of
/// it is `failed`, as Hyper aborts that message.
#[test]
fn admission_polled_on_another_task_is_judged_by_the_written_message() {
    let capture = Capture::new();
    let text = capture.block_on(async {
        let router = Router::new().route(
            "/",
            route(|admitted: AdmittedRequest| async move {
                admitted.into_streaming_response(budget(600, 300), |_scope| {
                    Body::from_stream(stream::iter([Ok::<_, Infallible>(Bytes::from_static(
                        b"hello",
                    ))]))
                })
            }),
        );
        let served = serve_composed(Composed::LowLevel(router, spawn_then_declare_ten, 1)).await;
        let mut client = served.request().await;
        let mut bytes = Vec::new();
        // Hyper aborts the message, so the read may end in a reset.
        let _ = timeout(WAIT, client.read_to_end(&mut bytes)).await.unwrap();
        wait_for_finished(&capture).await;
        served.shut_down().await;
        String::from_utf8_lossy(&bytes).into_owned()
    });
    assert_short_and_failed(&capture, &text);
}

/// Extensions kept from an earlier request by [`reuse_earlier_extensions`].
static EARLIER: std::sync::Mutex<Option<axum::http::Extensions>> = std::sync::Mutex::new(None);

/// A layer outside admission that keeps the first request's extensions, gives
/// them to the next request before admission, and rebuilds that request's
/// response around its body with a declared length of ten.
async fn reuse_earlier_extensions(
    mut request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let earlier = EARLIER.lock().unwrap().take();
    let Some(extensions) = earlier else {
        *EARLIER.lock().unwrap() = Some(request.extensions().clone());
        return next.run(request).await;
    };
    *request.extensions_mut() = extensions;
    let response = next.run(request).await;
    Response::builder()
        .header(header::CONTENT_LENGTH, "10")
        .body(response.into_body())
        .unwrap()
}

fn copy_then_rebuild(outside: Router) -> Router {
    outside.layer(axum::middleware::from_fn(reuse_earlier_extensions))
}

/// The registered server settles the body its own response carries, however
/// the request reached admission: here a layer replaced a later request's
/// extensions with an earlier request's and then rebuilt its response around
/// the body, declaring ten bytes the source falls short of.
#[test]
fn a_body_under_an_earlier_requests_extensions_is_settled_by_its_own_response() {
    *EARLIER.lock().unwrap() = None;
    let capture = Capture::new();
    let text = capture.block_on(async {
        let router = Router::new()
            .route("/first", route(|| async { "first" }))
            .route(
                "/",
                route(|admitted: AdmittedRequest| async move {
                    admitted.into_streaming_response(budget(600, 300), |_scope| {
                        Body::from_stream(stream::iter([Ok::<_, Infallible>(Bytes::from_static(
                            b"hello",
                        ))]))
                    })
                }),
            );
        let served = serve_composed(Composed::LowLevel(router, copy_then_rebuild, 1)).await;
        let mut first = TcpStream::connect(served.address).await.unwrap();
        first
            .write_all(b"GET /first HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();
        let mut answered = Vec::new();
        timeout(WAIT, first.read_to_end(&mut answered))
            .await
            .unwrap()
            .unwrap();
        assert!(answered.ends_with(b"first"), "{answered:?}");
        let mut client = served.request().await;
        let mut bytes = Vec::new();
        // Hyper aborts the message, so the read may end in a reset.
        let _ = timeout(WAIT, client.read_to_end(&mut bytes)).await.unwrap();
        wait_for_finished(&capture).await;
        served.shut_down().await;
        String::from_utf8_lossy(&bytes).into_owned()
    });
    assert_short_and_failed(&capture, &text);
}

/// A handler the registered server serves may call the in-process client and
/// keep the inner body rather than return it. The transport writing the outer
/// response never reaches that body, so it keeps the framing its own admission
/// settled, not the outer response's, even when it is read only after the
/// outer response was written.
#[test]
fn an_in_process_response_inside_a_served_handler_keeps_its_own_framing() {
    let capture = Capture::new();
    capture.block_on(async {
        let handle = super::super::support::ready();
        let inner = super::super::support::client(
            &handle,
            GuardedRouter::new().route(
                "/inner",
                route(|admitted: AdmittedRequest| async move {
                    admitted.into_streaming_response(budget(600, 300), |_scope| {
                        Body::from_stream(stream::iter([Ok::<_, Infallible>(Bytes::from_static(
                            b"a",
                        ))]))
                    })
                }),
            ),
        )
        .await;
        let (kept, later) = tokio::sync::oneshot::channel::<Body>();
        let kept = Arc::new(std::sync::Mutex::new(Some(kept)));
        let served = serve(GuardedRouter::new().route(
            "/",
            route(move || {
                let (inner, kept) = (inner.clone(), kept.clone());
                async move {
                    let request = axum::http::Request::get("/inner")
                        .body(Body::empty())
                        .unwrap();
                    let response = inner.request(request).await;
                    let _ = kept
                        .lock()
                        .unwrap()
                        .take()
                        .unwrap()
                        .send(response.into_body());
                    "ok"
                }
            }),
        ))
        .await;
        let mut client = served.request().await;
        let mut text = String::new();
        timeout(WAIT, client.read_to_string(&mut text))
            .await
            .unwrap()
            .unwrap();
        assert!(text.ends_with("\r\n\r\nok"), "{text}");
        // Only now, after the outer response was written, read the inner body.
        let mut body = later.await.unwrap();
        let (read, error) = super::super::support::drain(&mut body).await;
        assert_eq!((read.as_str(), error), ("a", None));
        drop(body);
        wait_for_finished(&capture).await;
        served.shut_down().await;
    });
    let event = assert_finished(&capture, "INFO", "completed");
    assert!(event.contains("bytes=1"), "{event}");
}

/// A served handler that returns an in-process response as its own hands that
/// body to the transport writing the outer message, which judges it: a length
/// the handler declares on it frames the message, and a source falling short
/// of it is `failed`, as Hyper aborts that message.
#[test]
fn an_in_process_response_a_served_handler_returns_is_judged_by_the_written_message() {
    let capture = Capture::new();
    let text = capture.block_on(async {
        let handle = super::super::support::ready();
        let inner = super::super::support::client(
            &handle,
            GuardedRouter::new().route(
                "/inner",
                route(|admitted: AdmittedRequest| async move {
                    admitted.into_streaming_response(budget(600, 300), |_scope| {
                        Body::from_stream(stream::iter([Ok::<_, Infallible>(Bytes::from_static(
                            b"hello",
                        ))]))
                    })
                }),
            ),
        )
        .await;
        let served = serve(GuardedRouter::new().route(
            "/",
            route(move || {
                let inner = inner.clone();
                async move {
                    let request = axum::http::Request::get("/inner")
                        .body(Body::empty())
                        .unwrap();
                    let mut response = inner.request(request).await;
                    response
                        .headers_mut()
                        .insert(header::CONTENT_LENGTH, HeaderValue::from_static("10"));
                    response
                }
            }),
        ))
        .await;
        let mut client = served.request().await;
        let mut bytes = Vec::new();
        // Hyper aborts the message, so the read may end in a reset.
        let _ = timeout(WAIT, client.read_to_end(&mut bytes)).await.unwrap();
        wait_for_finished(&capture).await;
        served.shut_down().await;
        String::from_utf8_lossy(&bytes).into_owned()
    });
    assert_short_and_failed(&capture, &text);
}
