use super::*;
use axum::{
    body::Body,
    http::{HeaderName, HeaderValue},
};

const LENGTH: &str = "content-length";
const ENCODING: &str = "transfer-encoding";

fn map(headers: &[(&'static str, &'static str)]) -> HeaderMap {
    let mut map = HeaderMap::new();
    for (name, value) in headers {
        map.append(
            HeaderName::from_static(name),
            HeaderValue::from_static(value),
        );
    }
    map
}

fn hinted(hint: Option<u64>) -> SizeHint {
    hint.map_or_else(SizeHint::default, SizeHint::with_exact)
}

fn settled(
    method: Method,
    version: Version,
    status: StatusCode,
    headers: &[(&'static str, &'static str)],
    hint: Option<u64>,
) -> Option<u64> {
    let headers = map(headers);
    let head = Head {
        method: &method,
        version,
        status,
        headers: &headers,
    };
    let framing = Framing::default();
    framing.settle(&head, &hinted(hint));
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

/// A body adopts the framing of the message whose transport calls into it,
/// once: admission's later settle and a second transport change nothing, and
/// outside a transport's call nothing is current.
#[test]
fn a_body_adopts_the_framing_of_the_message_whose_transport_reaches_it() {
    let start = BodyStart::new(&Request::new(Body::empty()));
    let (built, ()) = Response::new(()).into_parts();
    let framing = Framing::default();
    start.frame(&framing, &built, &SizeHint::with_exact(5));
    start.hand_over(&framing);
    framing.adopt_written();
    assert_eq!(framing.length(), Some(5));

    let ten = Written {
        length: Some(10),
        refused: false,
    };
    ten.writing(|| framing.adopt_written());
    assert_eq!(framing.length(), Some(10));
    assert!(!framing.unwritable());
    let declared = Response::builder()
        .header(LENGTH, "12")
        .body(Body::empty())
        .unwrap();
    start.settle(&declared);
    Written {
        length: None,
        refused: true,
    }
    .writing(|| framing.adopt_written());
    assert_eq!(framing.length(), Some(10));
    assert!(!framing.unwritable());
}

/// The written framing is current only for the transport's call: an inner
/// call restores the outer one, and an unwinding call restores it too.
#[test]
fn the_written_framing_is_current_only_during_the_transports_call() {
    let (outer, inner) = (
        Written {
            length: Some(1),
            refused: false,
        },
        Written {
            length: Some(2),
            refused: true,
        },
    );
    assert_eq!(Written::current(), None);
    outer.writing(|| {
        assert_eq!(Written::current(), Some(outer));
        inner.writing(|| assert_eq!(Written::current(), Some(inner)));
        assert_eq!(Written::current(), Some(outer));
        let unwound = std::panic::catch_unwind(|| inner.writing(|| panic!("unwinds")));
        assert!(unwound.is_err());
        assert_eq!(Written::current(), Some(outer));
    });
    assert_eq!(Written::current(), None);
}

/// A started body's framing follows the response admission returns; a body
/// whose start was refused keeps the framing of the response it was built into.
#[test]
fn the_returned_response_settles_a_started_body_again() {
    let start = BodyStart::new(&Request::new(Body::empty()));
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
        let start = BodyStart::new(&incoming);
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

/// The heads Hyper refuses to write for their headers: a `Content-Length`
/// beside `Transfer-Encoding`, unless `Transfer-Encoding` comes first in an
/// HTTP/1.0 answer, and, for a body of unknown size, an unparsable or
/// conflicting `Content-Length`.
#[test]
fn a_head_hyper_refuses_is_recognised() {
    let refused = |version, status, headers: &[(&'static str, &'static str)], hint: Option<u64>| {
        let headers = map(headers);
        let head = Head {
            method: &Method::GET,
            version,
            status,
            headers: &headers,
        };
        head.refused(Some(&hinted(hint)))
    };
    let (http11, http10, ok) = (Version::HTTP_11, Version::HTTP_10, StatusCode::OK);
    let encoding_first = [(ENCODING, "chunked"), (LENGTH, "10")];
    let length_first = [(LENGTH, "10"), (ENCODING, "chunked")];
    assert!(refused(http11, ok, &encoding_first, Some(10)));
    assert!(refused(http11, ok, &length_first, None));
    assert!(refused(http10, ok, &length_first, None));
    assert!(!refused(http10, ok, &encoding_first, None));
    assert!(refused(http11, ok, &[(LENGTH, "+5")], None));
    assert!(!refused(http11, ok, &[(LENGTH, "+5")], Some(5)));
    assert!(refused(http11, ok, &[(LENGTH, "10"), (LENGTH, "12")], None));
    assert!(!refused(http11, ok, &[(LENGTH, "10")], None));
    assert!(!refused(http11, ok, &[(ENCODING, "chunked")], Some(10)));
    // Hyper parses the headers before its no-body rule, so a bodyless status
    // refuses what it would refuse elsewhere, while a `Transfer-Encoding`
    // that comes first is dropped as for any answer that cannot be chunked.
    for status in [StatusCode::NOT_MODIFIED, StatusCode::NO_CONTENT] {
        assert!(!refused(http11, status, &encoding_first, None));
        assert!(refused(http11, status, &length_first, None));
        assert!(refused(http11, status, &[(LENGTH, "+5")], None));
    }
    // A 2xx answer to CONNECT may carry no length header at all.
    let connect = |headers: &[(&'static str, &'static str)]| {
        let headers = map(headers);
        let head = Head {
            method: &Method::CONNECT,
            version: http11,
            status: ok,
            headers: &headers,
        };
        head.refused(Some(&SizeHint::with_exact(0)))
    };
    assert!(connect(&[(LENGTH, "0")]));
    assert!(connect(&[(ENCODING, "chunked")]));
    assert!(!connect(&[]));
}

/// A body that has ended is written as none: Hyper skips every
/// `Content-Length` unparsed, so only a `Transfer-Encoding` before one in an
/// answer that may be chunked is refused, besides a final 1xx and any length
/// header on a 2xx answer to CONNECT. Answering HEAD, it writes the first
/// `Content-Length` as given and refuses a length header after it.
#[test]
fn an_ended_body_is_refused_only_as_hyper_writes_it_as_none() {
    let refused = |method: Method, version, status, headers: &[(&'static str, &'static str)]| {
        let headers = map(headers);
        let head = Head {
            method: &method,
            version,
            status,
            headers: &headers,
        };
        head.refused(None)
    };
    let (get, head) = (Method::GET, Method::HEAD);
    let (http11, http10, ok) = (Version::HTTP_11, Version::HTTP_10, StatusCode::OK);
    let length_first = [(LENGTH, "0"), (ENCODING, "chunked")];
    let encoding_first = [(ENCODING, "chunked"), (LENGTH, "0")];
    assert!(!refused(get.clone(), http10, ok, &length_first));
    assert!(!refused(get.clone(), http11, ok, &length_first));
    assert!(refused(get.clone(), http11, ok, &encoding_first));
    assert!(!refused(get.clone(), http10, ok, &encoding_first));
    assert!(!refused(
        get.clone(),
        http11,
        StatusCode::NO_CONTENT,
        &encoding_first
    ));
    assert!(!refused(get.clone(), http11, ok, &[(LENGTH, "+5")]));
    assert!(!refused(
        get.clone(),
        http11,
        ok,
        &[(LENGTH, "10"), (LENGTH, "12")]
    ));
    assert!(refused(get.clone(), http11, StatusCode::EARLY_HINTS, &[]));
    assert!(refused(Method::CONNECT, http11, ok, &[(LENGTH, "0")]));
    assert!(!refused(head.clone(), http11, ok, &[(LENGTH, "10")]));
    assert!(refused(
        head.clone(),
        http11,
        ok,
        &[(LENGTH, "10"), (LENGTH, "10")]
    ));
    assert!(refused(head.clone(), http11, ok, &length_first));
    assert!(!refused(head, http11, ok, &encoding_first));
}

/// Hyper writes no final 1xx but 101, answering any other with a 500 whatever
/// its headers, version or body.
#[test]
fn a_final_informational_head_is_refused_unless_switching_protocols() {
    let headers = HeaderMap::new();
    let refused = |status, version, hint: Option<u64>| {
        let head = Head {
            method: &Method::GET,
            version,
            status,
            headers: &headers,
        };
        head.refused(Some(&hinted(hint)))
    };
    for status in [
        StatusCode::CONTINUE,
        StatusCode::PROCESSING,
        StatusCode::EARLY_HINTS,
    ] {
        assert!(refused(status, Version::HTTP_11, None));
        assert!(refused(status, Version::HTTP_10, Some(0)));
    }
    assert!(!refused(
        StatusCode::SWITCHING_PROTOCOLS,
        Version::HTTP_11,
        None
    ));
}

/// A refusal comes only from the head the registered server hands Hyper,
/// which a body adopts with that message, never from the response the handler
/// built or admission returns, which may be discarded.
#[test]
fn only_the_written_head_records_a_refusal() {
    let start = BodyStart::new(&Request::new(Body::empty()));
    let refused = || {
        Response::builder()
            .header(LENGTH, "+5")
            .body(Body::from_stream(futures_util::stream::empty::<
                Result<axum::body::Bytes, std::convert::Infallible>,
            >()))
            .unwrap()
    };
    let framing = Framing::default();
    let (built, _) = refused().into_parts();
    start.frame(&framing, &built, &SizeHint::default());
    start.hand_over(&framing);
    start.settle(&refused());
    assert!(!framing.unwritable());
    // `Body::empty()` has ended, so Hyper writes it as none and skips its
    // length unparsed.
    let sized = Response::builder()
        .header(LENGTH, "+5")
        .body(Body::empty())
        .unwrap();
    let get = (&Method::GET, Version::HTTP_11);
    assert!(!Written::of(get.0, get.1, &sized).refused);
    let written = Written::of(get.0, get.1, &refused());
    assert_eq!(
        written,
        Written {
            length: None,
            refused: true,
        }
    );
    written.writing(|| framing.adopt_written());
    assert!(framing.unwritable());
}

/// Hyper writes no final 1xx but 101, whatever the body, and frames a HEAD
/// answer's declared length as nothing.
#[test]
fn the_written_framing_follows_the_request_head() {
    let early = Response::builder()
        .status(StatusCode::EARLY_HINTS)
        .body(Body::empty())
        .unwrap();
    assert!(Written::of(&Method::GET, Version::HTTP_11, &early).refused);
    let declared = Response::builder()
        .header(LENGTH, "10")
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        Written::of(&Method::HEAD, Version::HTTP_11, &declared).length,
        None
    );
    assert_eq!(
        Written::of(&Method::GET, Version::HTTP_10, &declared).length,
        Some(10)
    );
}
