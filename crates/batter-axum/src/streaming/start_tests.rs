use super::*;
use axum::{
    body::Body,
    http::{HeaderName, HeaderValue},
};

const LENGTH: &str = "content-length";
const ENCODING: &str = "transfer-encoding";

fn settled(
    method: Method,
    version: Version,
    status: StatusCode,
    headers: &[(&'static str, &'static str)],
    hint: Option<u64>,
) -> Option<u64> {
    let mut map = HeaderMap::new();
    for (name, value) in headers {
        map.append(
            HeaderName::from_static(name),
            HeaderValue::from_static(value),
        );
    }
    let hint = hint.map_or_else(SizeHint::default, SizeHint::with_exact);
    let framing = Framing::default();
    framing.settle(&method, version, status, &map, &hint);
    framing.length()
}

fn get(headers: &[(&'static str, &'static str)], hint: Option<u64>) -> Option<u64> {
    settled(Method::GET, Version::HTTP_11, StatusCode::OK, headers, hint)
}

/// Hyper frames by a `Content-Length` only when every value is plain digits
/// and all agree; otherwise the body's exact size frames it, if it has one. A
/// declared length the body's exact size contradicts, which Hyper does not
/// support, still counts: it is what the peer reads.
#[test]
fn a_content_length_frames_the_message_only_as_hyper_parses_it() {
    assert_eq!(get(&[(LENGTH, "10")], Some(4)), Some(10));
    assert_eq!(get(&[(LENGTH, "10"), (LENGTH, "10")], None), Some(10));
    for refused in ["+10", " 10", "", "1_0", "99999999999999999999"] {
        assert_eq!(get(&[(LENGTH, refused)], None), None, "{refused:?}");
        assert_eq!(get(&[(LENGTH, refused)], Some(4)), Some(4), "{refused:?}");
    }
    assert_eq!(get(&[(LENGTH, "10"), (LENGTH, "12")], None), None);
    assert_eq!(get(&[], Some(4)), Some(4));
    assert_eq!(get(&[], None), None);
}

/// A `Transfer-Encoding` header makes Hyper chunk-encode the message whatever
/// the body's size, except in an answer to HTTP/1.0, which drops the header.
#[test]
fn transfer_encoding_chunks_the_message_except_for_http_10() {
    assert_eq!(get(&[(ENCODING, "chunked")], Some(10)), None);
    assert_eq!(get(&[(ENCODING, "gzip")], Some(10)), None);
    assert_eq!(get(&[(LENGTH, "10"), (ENCODING, "chunked")], None), None);
    let http10 = |headers: &[(&'static str, &'static str)], hint| {
        settled(Method::GET, Version::HTTP_10, StatusCode::OK, headers, hint)
    };
    assert_eq!(
        http10(&[(ENCODING, "chunked"), (LENGTH, "10")], None),
        Some(10)
    );
    assert_eq!(http10(&[(ENCODING, "chunked")], Some(7)), Some(7));
    assert_eq!(http10(&[(ENCODING, "chunked")], None), None);
}

/// A response Hyper writes without a body frames no length.
#[test]
fn a_response_written_without_a_body_frames_nothing() {
    let length = [(LENGTH, "10")];
    let at = |method, status| settled(method, Version::HTTP_11, status, &length, Some(10));
    assert_eq!(at(Method::HEAD, StatusCode::OK), None);
    assert_eq!(at(Method::CONNECT, StatusCode::OK), None);
    assert_eq!(at(Method::GET, StatusCode::NO_CONTENT), None);
    assert_eq!(at(Method::GET, StatusCode::NOT_MODIFIED), None);
    assert_eq!(at(Method::GET, StatusCode::SWITCHING_PROTOCOLS), None);
    assert_eq!(at(Method::CONNECT, StatusCode::NOT_FOUND), Some(10));
}

/// Admission takes the registered server's record for its request, once: a
/// copy carried into another request gets a record of its own there.
#[test]
fn admission_claims_the_served_record_only_once() {
    let mut request = Request::new(Body::empty());
    let served = BodyStart::served(&request);
    request.extensions_mut().insert(served.clone());
    let claimed = BodyStart::claim(&mut request);
    assert!(Arc::ptr_eq(&served.0, &claimed.0));
    assert!(request.extensions().get::<BodyStart>().is_none());

    let mut copied = Request::new(Body::empty());
    copied.extensions_mut().insert(served.clone());
    assert!(!Arc::ptr_eq(&served.0, &BodyStart::claim(&mut copied).0));

    let mut unserved = Request::new(Body::empty());
    let own = BodyStart::claim(&mut unserved);
    assert!(lock(&own.0).claimed);
}

/// A started body's framing follows the response finally written; a refused
/// body keeps the framing of the response it was built into.
#[test]
fn the_written_response_settles_a_started_body_again() {
    let start = BodyStart::served(&Request::new(Body::empty()));
    let written = Response::builder()
        .header(LENGTH, "10")
        .body(Body::empty())
        .unwrap();
    let (parts, ()) = Response::new(()).into_parts();
    let built = |framing: &Framing| {
        start.frame(framing, &parts, &SizeHint::with_exact(5));
        assert_eq!(framing.length(), Some(5));
    };
    let refused = Framing::default();
    built(&refused);
    start.settle(&written);
    assert_eq!(refused.length(), Some(5));

    let started = Framing::default();
    built(&started);
    start.hand_over(&started);
    start.settle(&written);
    assert_eq!(started.length(), Some(10));
    assert_eq!(refused.length(), Some(5));
}

/// Hyper answers in HTTP/1.0 when the request was HTTP/1.0 and otherwise in
/// the response's own version, so an HTTP/1.0 response to an HTTP/1.1 request
/// drops `Transfer-Encoding` and is framed by its `Content-Length`.
#[test]
fn framing_follows_the_version_hyper_answers_in() {
    let answered = |request: Version, response: Version| {
        let mut incoming = Request::new(Body::empty());
        *incoming.version_mut() = request;
        let start = BodyStart::served(&incoming);
        let written = Response::builder()
            .version(response)
            .header(ENCODING, "chunked")
            .header(LENGTH, "10")
            .body(())
            .unwrap();
        let framing = Framing::default();
        start.frame(&framing, &written.into_parts().0, &SizeHint::default());
        framing.length()
    };
    assert_eq!(answered(Version::HTTP_11, Version::HTTP_10), Some(10));
    assert_eq!(answered(Version::HTTP_10, Version::HTTP_11), Some(10));
    assert_eq!(answered(Version::HTTP_11, Version::HTTP_11), None);
}
