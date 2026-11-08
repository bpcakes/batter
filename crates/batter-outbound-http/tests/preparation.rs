//! Origin, limit and request validation before any exchange exists.
//!
//! Nothing here connects: every refusal is reported by a constructor or by
//! `RequestDraft::prepare`.
#![allow(clippy::unwrap_used, reason = "a panic fails the test")]
#![allow(clippy::missing_assert_message, reason = "test names locate failures")]

use batter_outbound_http::{
    Client, ConnectTimeout, LimitError, Origin, OriginError, PrepareError, RequestDraft,
    ResponseLimit,
    http::{HeaderName, HeaderValue, Method, header},
};
use std::{collections::BTreeMap, net::SocketAddr, time::Duration};

fn client() -> Client {
    Client::new(
        Origin::https("https://provider.example").unwrap(),
        ConnectTimeout::new(Duration::from_secs(5)).unwrap(),
    )
    .unwrap()
}

#[test]
fn explicit_trailing_slash_does_not_admit_empty_or_dot_segments() {
    for segment in ["", ";x", ".", "..", "..;x", ".\t."] {
        assert!(
            client()
                .request(Method::GET, ["v1", segment])
                .trailing_slash()
                .prepare()
                .is_err(),
            "invalid segment {segment:?}"
        );
    }
}

fn refusal(draft: RequestDraft) -> Option<PrepareError> {
    draft.prepare().err()
}

fn text() -> HeaderValue {
    HeaderValue::from_static("text/plain")
}

#[test]
fn https_origins_accept_only_a_scheme_host_and_port() {
    for accepted in [
        "https://provider.example",
        "https://provider.example/",
        "https://provider.example:8443",
        "https://provider.example:443",
        "HTTPS://Provider.Example",
        "https://127.0.0.1:8443",
        "https://[::1]:8443",
    ] {
        assert!(Origin::https(accepted).is_ok(), "{accepted}");
    }
    for (refused, expected) in [
        ("provider.example", OriginError::Unparseable),
        ("", OriginError::Unparseable),
        ("https://", OriginError::Unparseable),
        ("https://provider.example:port", OriginError::Unparseable),
        ("http://provider.example", OriginError::NotHttps),
        ("ftp://provider.example", OriginError::NotHttps),
        ("wss://provider.example", OriginError::NotHttps),
        ("https://user@provider.example", OriginError::Userinfo),
        (
            "https://user:secret@provider.example",
            OriginError::Userinfo,
        ),
        ("https://:secret@provider.example", OriginError::Userinfo),
        ("https://provider.example:0", OriginError::Port),
        ("https://provider.example/v1", OriginError::Path),
        ("https://provider.example/v1/", OriginError::Path),
        ("https://provider.example//", OriginError::Path),
        ("https://provider.example?key=value", OriginError::Query),
        ("https://provider.example/?", OriginError::Query),
        ("https://provider.example#section", OriginError::Fragment),
        ("https://provider.example/#", OriginError::Fragment),
    ] {
        assert_eq!(Origin::https(refused), Err(expected), "{refused}");
    }
}

#[test]
fn loopback_origins_accept_only_a_loopback_address_with_a_port() {
    for accepted in ["127.0.0.1:8080", "127.0.0.2:8080", "[::1]:8080"] {
        let address: SocketAddr = accepted.parse().unwrap();
        assert!(Origin::loopback(address).is_ok(), "{accepted}");
    }
    for (refused, expected) in [
        ("192.0.2.10:8080", OriginError::NotLoopback),
        ("0.0.0.0:8080", OriginError::NotLoopback),
        ("[::]:8080", OriginError::NotLoopback),
        ("[::ffff:127.0.0.1]:8080", OriginError::NotLoopback),
        ("192.0.2.10:0", OriginError::NotLoopback),
        ("127.0.0.1:0", OriginError::Port),
        ("[::1]:0", OriginError::Port),
    ] {
        let address: SocketAddr = refused.parse().unwrap();
        assert_eq!(Origin::loopback(address), Err(expected), "{refused}");
    }
}

#[test]
fn limits_refuse_zero_and_values_above_their_maximum() {
    assert_eq!(ResponseLimit::MAX_BYTES, 64 * 1024 * 1024);
    assert_eq!(ResponseLimit::new(1).map(ResponseLimit::bytes), Ok(1));
    let maximum = ResponseLimit::new(ResponseLimit::MAX_BYTES).map(ResponseLimit::bytes);
    assert_eq!(maximum, Ok(ResponseLimit::MAX_BYTES));
    assert_eq!(ResponseLimit::new(0), Err(LimitError::Zero));
    let above = ResponseLimit::new(ResponseLimit::MAX_BYTES + 1);
    assert_eq!(above, Err(LimitError::AboveMaximum));

    assert_eq!(ConnectTimeout::MAX, Duration::from_secs(60));
    let shortest = Duration::from_nanos(1);
    assert_eq!(
        ConnectTimeout::new(shortest).map(ConnectTimeout::get),
        Ok(shortest)
    );
    let longest = ConnectTimeout::new(ConnectTimeout::MAX).map(ConnectTimeout::get);
    assert_eq!(longest, Ok(ConnectTimeout::MAX));
    assert_eq!(ConnectTimeout::new(Duration::ZERO), Err(LimitError::Zero));
    let above = ConnectTimeout::new(ConnectTimeout::MAX + Duration::from_nanos(1));
    assert_eq!(above, Err(LimitError::AboveMaximum));
}

#[test]
fn every_reserved_header_is_refused() {
    for name in [
        "Host",
        "Content-Length",
        "Transfer-Encoding",
        "Connection",
        "Upgrade",
        "TE",
        "Trailer",
        "Expect",
        "Keep-Alive",
        "Proxy-Connection",
        "Proxy-Authorization",
        "Content-Type",
    ] {
        let reserved = HeaderName::from_bytes(name.as_bytes()).unwrap();
        let draft = client()
            .request(Method::POST, ["v1"])
            .header(reserved, text());
        assert_eq!(refusal(draft), Some(PrepareError::ReservedHeader), "{name}");
    }
    for name in [
        "authorization",
        "accept",
        "user-agent",
        "idempotency-key",
        "x-request-ref",
    ] {
        let ordinary = HeaderName::from_bytes(name.as_bytes()).unwrap();
        let draft = client()
            .request(Method::POST, ["v1"])
            .header(ordinary, text());
        assert_eq!(refusal(draft), None, "{name}");
    }
}

#[test]
fn a_second_body_is_refused() {
    let json = serde_json::json!({ "effect": "example" });
    let drafts = [
        client()
            .request(Method::POST, ["v1"])
            .json(&json)
            .body(text(), "x"),
        client()
            .request(Method::POST, ["v1"])
            .body(text(), "x")
            .json(&json),
        client()
            .request(Method::POST, ["v1"])
            .body(text(), "x")
            .body(text(), "y"),
        client()
            .request(Method::POST, ["v1"])
            .json(&json)
            .json(&json),
    ];
    for draft in drafts {
        assert_eq!(refusal(draft), Some(PrepareError::BodyAlreadySet));
    }
}

#[test]
fn a_json_serialization_failure_is_reported_at_preparation() {
    // JSON object keys must be strings; a tuple key cannot be serialized.
    let unserializable = BTreeMap::from([((1_u8, 2_u8), "value")]);
    let draft = client().request(Method::POST, ["v1"]).json(&unserializable);
    assert_eq!(refusal(draft), Some(PrepareError::Json));
}

#[test]
fn preparation_reports_the_first_draft_problem() {
    let unserializable = BTreeMap::from([((1_u8, 2_u8), "value")]);
    let draft = client()
        .request(Method::POST, ["v1"])
        .header(header::HOST, text())
        .body(text(), "x")
        .body(text(), "y");
    assert_eq!(refusal(draft), Some(PrepareError::ReservedHeader));
    let draft = client()
        .request(Method::POST, ["v1"])
        .body(text(), "x")
        .body(text(), "y")
        .header(header::HOST, text());
    assert_eq!(refusal(draft), Some(PrepareError::BodyAlreadySet));
    let draft = client()
        .request(Method::POST, ["v1"])
        .json(&unserializable)
        .header(header::HOST, text());
    assert_eq!(refusal(draft), Some(PrepareError::Json));
}

#[test]
fn connect_is_refused_and_other_methods_are_accepted() {
    let draft = client().request(Method::CONNECT, ["v1"]);
    assert_eq!(refusal(draft), Some(PrepareError::ConnectMethod));
    let extension = Method::from_bytes(b"PURGE").unwrap();
    for method in [
        Method::GET,
        Method::HEAD,
        Method::POST,
        Method::PUT,
        Method::PATCH,
        Method::DELETE,
        Method::OPTIONS,
        extension,
    ] {
        let draft = client().request(method.clone(), ["v1"]);
        assert_eq!(refusal(draft), None, "{method}");
    }
}

#[test]
fn empty_and_dot_segments_are_refused() {
    for (segments, expected) in [
        (&[""][..], PrepareError::EmptySegment),
        (&["v1", ""], PrepareError::EmptySegment),
        (&["", "v1"], PrepareError::EmptySegment),
        (&["."], PrepareError::DotSegment),
        (&[".."], PrepareError::DotSegment),
        (&["v1", ".", "items"], PrepareError::DotSegment),
        (&["v1", ".."], PrepareError::DotSegment),
        // Servers that strip `;` path parameters would read these as above.
        (&["..;"], PrepareError::DotSegment),
        (&["v1", "..;x=1", "admin"], PrepareError::DotSegment),
        (&[".;x"], PrepareError::DotSegment),
        (&[";x"], PrepareError::EmptySegment),
    ] {
        let draft = client().request(Method::GET, segments.iter().copied());
        assert_eq!(refusal(draft), Some(expected), "{segments:?}");
    }
    // Only exact dot segments are refused; these stay ordinary segments.
    for segments in [
        &["..."][..],
        &[".hidden"],
        &["%2e%2e"],
        &["a.."],
        &["a;b"],
        &["...;x"],
        &[],
    ] {
        let draft = client().request(Method::GET, segments.iter().copied());
        assert_eq!(refusal(draft), None, "{segments:?}");
    }
}

#[test]
fn segments_that_url_parsing_would_rewrite_are_refused() {
    // URL parsing deletes tab, line feed and carriage return before applying
    // its dot-segment rules, so ".\t." would otherwise remove a path level.
    for segments in [
        &[".\t."][..],
        &["v1", "accounts", ".\t.", "admin"],
        &["\t"],
        &["a\nb"],
        &["a\rb"],
        &["v1", "trailing\n"],
    ] {
        let draft = client().request(Method::GET, segments.iter().copied());
        assert_eq!(
            refusal(draft),
            Some(PrepareError::ControlCharacter),
            "{segments:?}"
        );
    }
    // Other control characters are percent-encoded and stay inside the segment.
    let draft = client().request(Method::GET, ["a\u{1}b", "\u{7f}"]);
    assert_eq!(refusal(draft), None);
}

#[test]
fn urls_that_are_not_http_uris_are_refused_before_an_exchange_exists() {
    // reqwest would refuse these only when the request is executed.
    let long_segment = "a".repeat(70 * 1024);
    let draft = client().request(Method::GET, [long_segment.as_str()]);
    assert_eq!(refusal(draft), Some(PrepareError::InvalidUri));
    let long_value = "b".repeat(70 * 1024);
    let draft = client()
        .request(Method::GET, ["v1"])
        .query("key", &long_value);
    assert_eq!(refusal(draft), Some(PrepareError::InvalidUri));
    // A host that URL parsing accepts but an HTTP URI does not.
    assert_eq!(
        Origin::https("https://{{hostname}}/"),
        Err(OriginError::Unparseable)
    );
}
