//! A body the library ends at a bound while Hyper is writing its HTTP/1.1
//! message: what the peer then holds, without and with a declared length.

use super::*;

/// Serve an HTTP/1.1 request a body that yields six bytes and then stays
/// pending, so the library's idle bound ends it, declaring `length` inside
/// the builder when given. Returns what the peer read: first up to those six
/// bytes, then whatever follows until the connection ends.
fn served_until_the_idle_bound(capture: &Capture, length: Option<&'static str>) -> String {
    capture.block_on(async {
        let served = serve(GuardedRouter::new().route(
            "/",
            route(move |admitted: AdmittedRequest| async move {
                let budget =
                    BodyBudget::new(Duration::from_secs(600), Duration::from_millis(200)).unwrap();
                admitted.into_streaming_response(budget, move |_scope| {
                    let mut headers = HeaderMap::new();
                    if let Some(length) = length {
                        headers.insert(header::CONTENT_LENGTH, HeaderValue::from_static(length));
                    }
                    (headers, stuck())
                })
            }),
        ))
        .await;
        let mut client = served.request().await;
        let mut bytes = Vec::new();
        timeout(WAIT, async {
            while !String::from_utf8_lossy(&bytes).contains("first;") {
                assert_ne!(client.read_buf(&mut bytes).await.unwrap(), 0);
            }
        })
        .await
        .unwrap();
        // Hyper aborts the message, so the read may end in a reset.
        let _ = timeout(WAIT, client.read_to_end(&mut bytes)).await.unwrap();
        wait_for_finished(capture).await;
        served.shut_down().await;
        String::from_utf8_lossy(&bytes).into_owned()
    })
}

/// Without a declared length Hyper sends the body chunked. The idle bound
/// ends it with an error after its one chunk, and the connection ends
/// without the final chunk that would complete the message.
#[test]
fn an_interrupted_chunked_body_reaches_an_http11_peer_without_its_final_chunk() {
    let capture = Capture::new();
    let text = served_until_the_idle_bound(&capture, None);
    let (head, body) = text.split_once("\r\n\r\n").unwrap();
    assert!(head.starts_with("HTTP/1.1 200 "), "{text}");
    assert!(declares(head, "transfer-encoding: chunked"), "{text}");
    assert_eq!(body, "6\r\nfirst;\r\n", "{text}");
    let event = assert_finished(&capture, "WARN", "idle_timeout");
    assert!(event.contains("bytes=6"), "{event}");
}

/// With a ten-byte length declared inside the builder Hyper frames the
/// message by it. The idle bound ends the body with an error after six
/// bytes, and the connection ends four bytes short of the declaration.
#[test]
fn an_interrupted_body_reaches_an_http11_peer_short_of_its_declared_length() {
    let capture = Capture::new();
    let text = served_until_the_idle_bound(&capture, Some("10"));
    let (head, body) = text.split_once("\r\n\r\n").unwrap();
    assert!(head.starts_with("HTTP/1.1 200 "), "{text}");
    assert!(declares(head, "content-length: 10"), "{text}");
    assert!(!declares(head, "transfer-encoding: chunked"), "{text}");
    assert_eq!(body, "first;", "{text}");
    let event = assert_finished(&capture, "WARN", "idle_timeout");
    assert!(event.contains("bytes=6"), "{event}");
}
