//! What reaches the peer: the client profile, request encoding and redirects.
//!
//! These tests read the raw bytes the loopback peer received, so they check
//! the request as sent rather than as configured.
#![allow(clippy::unwrap_used, reason = "a panic fails the test")]
#![allow(clippy::missing_assert_message, reason = "test names locate failures")]

#[allow(dead_code, reason = "shared support; this crate uses part of it")]
#[path = "support/peer.rs"]
mod peer;

use batter_outbound_http::{
    Body, Client, Exchange, RequestDraft,
    http::{HeaderName, HeaderValue, Method, StatusCode, header},
};
use peer::{Peer, Recorded, Step, after, limit, write};
use std::{process::Command, time::Duration};

const OK: &str = "HTTP/1.1 200 OK\r\ncontent-length: 6\r\n\r\norigin";

/// Send the request built by `draft` to a peer that answers `OK`, and return
/// the only request the peer received.
async fn sent(draft: impl FnOnce(&Client) -> RequestDraft) -> (Peer, Recorded) {
    let peer = Peer::start([Step::Read, write(OK)]).await;
    let prepared = draft(&peer.client()).prepare().expect("a valid request");
    let outcome = prepared
        .send(after(Duration::from_secs(2)), limit(1024))
        .await;
    assert!(
        matches!(&outcome, Exchange::Responded(response) if response.status() == StatusCode::OK),
        "{outcome:?}"
    );
    let request = peer.single_request().await;
    (peer, request)
}

/// Test builds compile reqwest's gzip and HTTP/2 support through this
/// package's dev-dependencies, so the missing `accept-encoding` shows that the
/// profile turns gzip off (tests/tls_hello.rs pins the HTTP/1.1-only ALPN).
/// Brotli, zstd and deflate are not compiled here. `Referer` and
/// `Proxy-Authorization` would only appear on a redirect or through a proxy,
/// so their absence is wire shape, not evidence for those settings.
#[tokio::test]
async fn requests_arrive_as_http1_origin_form_without_referer_or_accept_encoding() {
    let (peer, request) = sent(|client| client.request(Method::GET, ["v1", "items"])).await;
    assert_eq!(request.request_line(), "GET /v1/items HTTP/1.1");
    assert_eq!(request.header("host"), [peer.address().to_string()]);
    for absent in [
        "referer",
        "accept-encoding",
        "proxy-authorization",
        "te",
        "upgrade",
    ] {
        assert!(
            request.header(absent).is_empty(),
            "{absent} sent: {request:?}"
        );
    }
}

#[tokio::test]
async fn header_values_reach_the_peer_unchanged() {
    let tag = HeaderName::from_static("x-tag");
    let (_peer, request) = sent(|client| {
        client
            .request(Method::GET, ["v1"])
            .header(
                header::AUTHORIZATION,
                HeaderValue::from_static("Bearer t-1"),
            )
            .header(tag.clone(), HeaderValue::from_static("first"))
            .header(tag.clone(), HeaderValue::from_static("second"))
    })
    .await;
    assert_eq!(request.header("authorization"), ["Bearer t-1"]);
    assert_eq!(request.header("x-tag"), ["first", "second"]);
}

#[tokio::test]
async fn segments_are_percent_encoded_inside_one_segment() {
    let (_peer, request) =
        sent(|client| client.request(Method::GET, ["v1", "a/b?c#d%e\\f g", "%2e%2e", "...", "é"]))
            .await;
    assert_eq!(
        request.request_line(),
        "GET /v1/a%2Fb%3Fc%23d%25e%5Cf%20g/%252e%252e/.../%C3%A9 HTTP/1.1"
    );
}

#[tokio::test]
async fn an_empty_path_targets_the_origin_root() {
    let (_peer, request) = sent(|client| client.request(Method::GET, [])).await;
    assert_eq!(request.request_line(), "GET / HTTP/1.1");
}

#[tokio::test]
async fn query_pairs_are_form_encoded_in_call_order() {
    let (_peer, request) = sent(|client| {
        client
            .request(Method::GET, ["v1", "search"])
            .query("note", "a b&c=d+e/?#%")
            .query("k", "1")
            .query("k", "2")
    })
    .await;
    assert_eq!(
        request.request_line(),
        "GET /v1/search?note=a+b%26c%3Dd%2Be%2F%3F%23%25&k=1&k=2 HTTP/1.1"
    );
}

#[tokio::test]
async fn bodies_carry_their_content_type_and_length() {
    let value = serde_json::json!({ "effect": "example", "count": 2 });
    let expected = serde_json::to_vec(&value).unwrap();
    let (_peer, json) = sent(|client| client.request(Method::POST, ["v1"]).json(&value)).await;
    assert_eq!(json.header("content-type"), ["application/json"]);
    assert_eq!(json.header("content-length"), [expected.len().to_string()]);
    assert_eq!(json.body(), &expected[..]);
    let plain = HeaderValue::from_static("text/plain; charset=utf-8");
    let (_peer, raw) = sent(|client| {
        client
            .request(Method::PUT, ["v1"])
            .body(plain, "plain body")
    })
    .await;
    assert_eq!(raw.request_line(), "PUT /v1 HTTP/1.1");
    assert_eq!(raw.header("content-type"), ["text/plain; charset=utf-8"]);
    assert_eq!(raw.body(), b"plain body");
}

#[tokio::test]
async fn redirect_is_returned_and_never_followed() {
    let peer = Peer::start([
        Step::Read,
        write("HTTP/1.1 307 Temporary Redirect\r\nlocation: /second\r\ncontent-length: 0\r\n\r\n"),
    ])
    .await;
    let prepared = peer
        .client()
        .request(Method::POST, ["first"])
        .header(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer t-1"),
        )
        .json(&serde_json::json!({ "effect": "example" }))
        .prepare()
        .unwrap();
    let outcome = prepared
        .send(after(Duration::from_secs(2)), limit(1024))
        .await;
    let Exchange::Responded(response) = outcome else {
        panic!("{outcome:?}");
    };
    assert_eq!(response.status(), StatusCode::TEMPORARY_REDIRECT);
    let location = &response.headers()[header::LOCATION];
    assert_eq!(location, "/second");
    assert!(location.is_sensitive());
    assert!(matches!(response.body(), Body::Complete(bytes) if bytes.is_empty()));
    // A followed redirect is sent before the response returns.
    let request = peer.single_request().await;
    assert_eq!(request.request_line(), "POST /first HTTP/1.1");
}

#[tokio::test]
async fn clones_share_one_connection_pool() {
    let peer = Peer::start([Step::Read, write(OK), Step::Read, write(OK)]).await;
    let first = peer.client();
    let second = first.clone();
    for client in [first, second] {
        let prepared = client.request(Method::GET, ["v1"]).prepare().unwrap();
        let outcome = prepared
            .send(after(Duration::from_secs(2)), limit(1024))
            .await;
        assert_eq!(outcome.label(), "responded");
        // Let the finished connection return to the idle pool.
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(peer.requests().len(), 2);
    assert_eq!(peer.connections(), 1);
}

#[tokio::test]
async fn idle_connections_are_closed_before_common_server_keep_alive_timeouts() {
    // The peer would serve both requests on one connection, as the test above
    // shows. After more than the four-second pool idle timeout, the client opens
    // a new connection instead of writing to one a server may be closing.
    let peer = Peer::start([Step::Read, write(OK), Step::Read, write(OK)]).await;
    let client = peer.client();
    for pause in [Duration::from_millis(4_500), Duration::ZERO] {
        let prepared = client.request(Method::GET, ["v1"]).prepare().unwrap();
        let outcome = prepared
            .send(after(Duration::from_secs(2)), limit(1024))
            .await;
        assert_eq!(outcome.label(), "responded");
        tokio::time::sleep(pause).await;
    }
    assert_eq!(peer.requests().len(), 2);
    assert_eq!(peer.connections(), 2);
}

const PROXY_TEST: &str = "proxy_environment_variables_are_ignored";
const PROXY_CHILD: &str = "BATTER_OUTBOUND_PROXY_CHILD";
const PROXY_VARIABLES: [&str; 8] = [
    "HTTP_PROXY",
    "http_proxy",
    "HTTPS_PROXY",
    "https_proxy",
    "ALL_PROXY",
    "all_proxy",
    "NO_PROXY",
    "no_proxy",
];

/// The parent sets proxy variables only for a child running this test, so
/// no other test sees them. In the child, a default reqwest client is the
/// control that shows the variables take effect.
#[tokio::test]
async fn proxy_environment_variables_are_ignored() {
    if std::env::var_os(PROXY_CHILD).is_some() {
        return proxy_child().await;
    }
    let proxy = Peer::start([
        Step::Read,
        write("HTTP/1.1 200 OK\r\ncontent-length: 5\r\n\r\nproxy"),
    ])
    .await;
    let proxy_url = format!("http://{}", proxy.address());
    let mut child = Command::new(std::env::current_exe().expect("test executable"));
    child.args(["--exact", PROXY_TEST, "--nocapture", "--test-threads=1"]);
    for name in PROXY_VARIABLES {
        child.env_remove(name);
    }
    child
        .env(PROXY_CHILD, "1")
        .env_remove("REQUEST_METHOD")
        .env("HTTP_PROXY", &proxy_url)
        .env("http_proxy", &proxy_url);
    let output = tokio::task::spawn_blocking(move || child.output())
        .await
        .expect("child joined")
        .expect("child started");
    assert!(output.status.success(), "child failed: {output:?}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("1 passed"), "child ran no test: {stdout}");
    // Only the control went through the proxy, in proxy (absolute) form.
    let control = proxy.single_request().await;
    let line = control.request_line();
    assert!(line.starts_with("GET http://127.0.0.1:"), "{line}");
    assert!(line.ends_with("/control HTTP/1.1"), "{line}");
}

async fn proxy_child() {
    let origin = Peer::start([Step::Read, write(OK)]).await;
    // The control is a default reqwest client. This workspace compiles reqwest
    // without a crypto provider, so the child process installs one for it; the
    // package's own client never uses the process default.
    let _ = rustls::crypto::ring::default_provider().install_default();
    let control = reqwest::Client::new()
        .get(format!("http://{}/control", origin.address()))
        .send()
        .await
        .expect("the control reaches the proxy");
    assert_eq!(control.bytes().await.unwrap(), "proxy");
    assert_eq!(origin.connections(), 0, "the control bypassed the proxy");
    let prepared = origin
        .client()
        .request(Method::GET, ["subject"])
        .prepare()
        .unwrap();
    let outcome = prepared
        .send(after(Duration::from_secs(2)), limit(1024))
        .await;
    let Exchange::Responded(response) = outcome else {
        panic!("{outcome:?}");
    };
    assert_eq!(response.body().complete().unwrap(), "origin");
    assert_eq!(
        origin.single_request().await.request_line(),
        "GET /subject HTTP/1.1"
    );
}
