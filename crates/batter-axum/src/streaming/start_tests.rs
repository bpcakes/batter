use super::*;
use axum::{body::Body, http::HeaderValue, response::Response};

fn declared(
    method: Method,
    status: StatusCode,
    headers: &[(&'static str, &'static str)],
    exact: Option<u64>,
) -> Option<u64> {
    let mut request = Request::new(Body::empty());
    *request.method_mut() = method;
    let mut built = Response::new(());
    *built.status_mut() = status;
    for (name, value) in headers {
        built
            .headers_mut()
            .append(*name, HeaderValue::from_static(value));
    }
    BodyStart::new(&request).declared(&built.into_parts().0, || exact)
}

/// The body's size is asked only where it decides: not for a response
/// without content, and not beside a `Content-Length` that states a length.
#[test]
fn the_bodys_size_is_asked_only_where_nothing_else_decides() {
    let asked = |method: Method, status: StatusCode, headers: &[(&'static str, &'static str)]| {
        let mut request = Request::new(Body::empty());
        *request.method_mut() = method;
        let mut built = Response::new(());
        *built.status_mut() = status;
        for (name, value) in headers {
            built
                .headers_mut()
                .append(*name, HeaderValue::from_static(value));
        }
        let asked = std::cell::Cell::new(0);
        BodyStart::new(&request).declared(&built.into_parts().0, || {
            asked.set(asked.get() + 1);
            Some(5)
        });
        asked.get()
    };
    assert_eq!(
        asked(Method::GET, StatusCode::OK, &[("content-length", "10")]),
        0
    );
    assert_eq!(asked(Method::HEAD, StatusCode::OK, &[]), 0);
    assert_eq!(asked(Method::GET, StatusCode::NO_CONTENT, &[]), 0);
    assert_eq!(asked(Method::GET, StatusCode::OK, &[]), 1);
    assert_eq!(
        asked(Method::GET, StatusCode::OK, &[("content-length", "+10")]),
        1
    );
}

fn get(headers: &[(&'static str, &'static str)], exact: Option<u64>) -> Option<u64> {
    declared(Method::GET, StatusCode::OK, headers, exact)
}

/// A `Content-Length` declares a length only as plain digits, repeated only
/// with one value; anything else leaves the body's exact size, if it has one.
#[test]
fn a_content_length_declares_a_length_only_as_one_plain_number() {
    const LENGTH: &str = "content-length";
    assert_eq!(get(&[(LENGTH, "10")], None), Some(10));
    assert_eq!(get(&[(LENGTH, "10")], Some(5)), Some(10));
    assert_eq!(get(&[(LENGTH, "0")], Some(5)), Some(0));
    assert_eq!(get(&[(LENGTH, "10"), (LENGTH, "10")], None), Some(10));
    assert_eq!(get(&[(LENGTH, "10"), (LENGTH, "11")], None), None);
    assert_eq!(get(&[(LENGTH, "10"), (LENGTH, "11")], Some(5)), Some(5));
    for malformed in ["+10", " 10", "10 ", "1_0", "", "10, 10", "0x10"] {
        assert_eq!(get(&[(LENGTH, malformed)], None), None, "{malformed:?}");
        assert_eq!(get(&[(LENGTH, malformed)], Some(5)), Some(5));
    }
    // One past `u64::MAX` overflows and declares nothing.
    assert_eq!(
        get(&[(LENGTH, "18446744073709551615")], None),
        Some(u64::MAX)
    );
    assert_eq!(get(&[(LENGTH, "18446744073709551616")], None), None);
}

/// Without a `Content-Length` the body's exact size is the declaration, and
/// a body of unknown size declares none. No other header takes part.
#[test]
fn the_bodys_exact_size_declares_a_length_without_a_header() {
    assert_eq!(get(&[], Some(5)), Some(5));
    assert_eq!(get(&[], None), None);
    assert_eq!(get(&[("transfer-encoding", "chunked")], Some(5)), Some(5));
    assert_eq!(
        get(
            &[("transfer-encoding", "chunked"), ("content-length", "10")],
            None
        ),
        Some(10)
    );
}

/// A response that carries no content declares no length for its body,
/// whatever its headers or its body's size say.
#[test]
fn a_response_without_content_declares_no_length() {
    let length: &[(&'static str, &'static str)] = &[("content-length", "10")];
    for status in [
        StatusCode::CONTINUE,
        StatusCode::SWITCHING_PROTOCOLS,
        StatusCode::EARLY_HINTS,
        StatusCode::NO_CONTENT,
        StatusCode::NOT_MODIFIED,
    ] {
        assert_eq!(declared(Method::GET, status, length, Some(10)), None);
    }
    for status in [StatusCode::OK, StatusCode::NOT_FOUND] {
        assert_eq!(declared(Method::HEAD, status, length, Some(10)), None);
        assert_eq!(declared(Method::GET, status, length, None), Some(10));
        assert_eq!(declared(Method::POST, status, &[], Some(7)), Some(7));
    }
    assert_eq!(
        declared(Method::CONNECT, StatusCode::OK, length, None),
        None
    );
    // A refused CONNECT is an ordinary response with content.
    assert_eq!(
        declared(Method::CONNECT, StatusCode::FORBIDDEN, length, None),
        Some(10)
    );
}
