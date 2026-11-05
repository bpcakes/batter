//! Request, response and refused-input values never reach Debug or Display.
//!
//! Each place a value can enter carries its own marker. The peer's raw
//! request shows that the markers were really sent, so their absence from
//! formatted output is redaction, not loss.
#![allow(clippy::unwrap_used, reason = "a panic fails the test")]
#![allow(clippy::missing_assert_message, reason = "test names locate failures")]

#[allow(dead_code, reason = "shared support; this crate uses part of it")]
#[path = "support/peer.rs"]
mod peer;

use batter_outbound_http::{
    Client, Exchange, Origin, PrepareError, RequestDraft,
    http::{HeaderName, HeaderValue, Method, header},
};
use peer::{Peer, RefusingPort, Step, after, connect_timeout, limit, millis, write};
use serde::ser::{Error as _, Serialize, Serializer};
use std::{fmt::Debug, time::Duration};

const HOST: &str = "host-marker-61";
const SEGMENT: &str = "segment-marker-72";
const QUERY: &str = "query-marker-83";
const HEADER: &str = "header-marker-94";
const BODY: &str = "body-marker-a5";
const RESPONSE: &str = "response-marker-b6";
const MARKERS: [&str; 6] = [HOST, SEGMENT, QUERY, HEADER, BODY, RESPONSE];

fn assert_hidden(rendered: &str) {
    for marker in MARKERS {
        assert!(!rendered.contains(marker), "{marker} leaked: {rendered}");
    }
}

fn assert_debug_hidden(value: &impl Debug) {
    assert_hidden(&format!("{value:?}"));
    assert_hidden(&format!("{value:#?}"));
}

fn assert_error_hidden(error: &impl std::error::Error) {
    assert_hidden(&error.to_string());
    assert_debug_hidden(error);
}

/// A request carrying a marker in every value it can hold.
fn marked(client: &Client) -> RequestDraft {
    client
        .request(Method::POST, ["v1", SEGMENT])
        .query("reference", QUERY)
        .header(header::AUTHORIZATION, HeaderValue::from_static(HEADER))
        .body(HeaderValue::from_static("text/plain"), BODY)
}

/// Fails with a marker in the serializer's own message.
struct Unserializable;

impl Serialize for Unserializable {
    fn serialize<S: Serializer>(&self, _: S) -> Result<S::Ok, S::Error> {
        Err(S::Error::custom(BODY))
    }
}

#[test]
fn client_origin_draft_and_prepared_request_show_no_values() {
    let origin = Origin::https(&format!("https://{HOST}.provider.example")).unwrap();
    assert_eq!(format!("{origin:?}"), "Origin(https)");
    let loopback = Origin::loopback("127.0.0.1:8080".parse().unwrap()).unwrap();
    assert_eq!(format!("{loopback:?}"), "Origin(loopback http)");
    let client = Client::new(origin, connect_timeout(Duration::from_secs(5))).unwrap();
    assert_eq!(format!("{client:?}"), "Client { .. }");
    let draft = marked(&client);
    assert_eq!(format!("{draft:?}"), "RequestDraft { .. }");
    let prepared = draft.prepare().unwrap();
    assert_eq!(format!("{prepared:?}"), "PreparedRequest { .. }");
    assert_eq!(format!("{prepared:#?}"), "PreparedRequest { .. }");
}

#[test]
fn origin_errors_do_not_echo_the_refused_input() {
    for input in [
        format!("https://user:{HOST}@provider.example"),
        format!("https://{HOST}@provider.example"),
        format!("https://provider.example/{HOST}"),
        format!("https://provider.example/?{HOST}=1"),
        format!("https://provider.example/#{HOST}"),
        format!("http://{HOST}.example"),
        format!("https://{HOST}.example:0"),
        format!("https://{HOST}.example:port"),
        HOST.to_owned(),
    ] {
        let error = Origin::https(&input).expect_err(&input);
        assert_error_hidden(&error);
    }
}

#[test]
fn preparation_errors_do_not_echo_request_values() {
    let client = Client::new(
        Origin::https("https://provider.example").unwrap(),
        connect_timeout(Duration::from_secs(5)),
    )
    .unwrap();
    let refusals = [
        marked(&client).header(
            header::PROXY_AUTHORIZATION,
            HeaderValue::from_static(HEADER),
        ),
        marked(&client).body(HeaderValue::from_static("text/plain"), BODY),
        client
            .request(Method::POST, [SEGMENT])
            .json(&Unserializable),
        client.request(Method::GET, [SEGMENT, ".."]),
        client.request(Method::GET, [SEGMENT, ""]),
        client
            .request(Method::CONNECT, [SEGMENT])
            .query("reference", QUERY),
    ];
    let mut seen = Vec::new();
    for draft in refusals {
        let error = draft.prepare().expect_err("refused");
        assert_error_hidden(&error);
        seen.push(error);
    }
    assert_eq!(
        seen,
        [
            PrepareError::ReservedHeader,
            PrepareError::BodyAlreadySet,
            PrepareError::Json,
            PrepareError::DotSegment,
            PrepareError::EmptySegment,
            PrepareError::ConnectMethod,
        ]
    );
}

#[tokio::test]
async fn outcomes_without_a_head_show_no_request_values() {
    let silent = Peer::start([Step::Read, Step::Hold]).await;
    let expired = marked(&silent.client()).prepare().unwrap();
    let not_started = expired.send(after(Duration::ZERO), limit(64)).await;
    assert_eq!(not_started.label(), "not_started.deadline");
    assert_debug_hidden(&not_started);

    let refusing = RefusingPort::bind().await;
    let not_dispatched = marked(&refusing.client())
        .prepare()
        .unwrap()
        .send(after(Duration::from_secs(2)), limit(64))
        .await;
    assert_eq!(not_dispatched.label(), "not_dispatched.connect_failed");
    assert_debug_hidden(&not_dispatched);

    let deadline = marked(&silent.client())
        .prepare()
        .unwrap()
        .send(after(millis(400)), limit(64))
        .await;
    assert_eq!(deadline.label(), "unknown.deadline");
    assert_debug_hidden(&deadline);
    assert_request_carried_markers(&silent).await;

    let closing = Peer::start([Step::Read]).await;
    let transport = marked(&closing.client())
        .prepare()
        .unwrap()
        .send(after(Duration::from_secs(2)), limit(64))
        .await;
    assert_eq!(transport.label(), "unknown.transport");
    assert_debug_hidden(&transport);
    assert_request_carried_markers(&closing).await;
}

/// The control: every request marker was on the wire.
async fn assert_request_carried_markers(peer: &Peer) {
    let request = peer.single_request().await;
    for marker in [SEGMENT, QUERY, HEADER, BODY] {
        assert!(
            request.contains(marker),
            "{marker} was not sent: {request:?}"
        );
    }
}

#[tokio::test]
async fn responded_outcomes_show_no_received_values() {
    let length = RESPONSE.len();
    let head = format!(
        "HTTP/1.1 200 OK\r\nx-echo: {RESPONSE}\r\nset-cookie: s={RESPONSE}\r\n\
         content-length: {length}\r\n\r\n"
    );
    let whole = format!("{head}{RESPONSE}");
    let short = format!("{head}{}", &RESPONSE[..length / 2]);
    let cases = [
        (whole.clone(), length, "responded"),
        (whole, length - 1, "responded.body_too_large"),
        (short, length, "responded.body_transport"),
    ];
    for (script, bytes, label) in cases {
        let peer = Peer::start([Step::Read, write(script)]).await;
        let outcome = marked(&peer.client())
            .prepare()
            .unwrap()
            .send(after(Duration::from_secs(2)), limit(bytes))
            .await;
        assert_eq!(outcome.label(), label);
        assert_debug_hidden(&outcome);
        let Exchange::Responded(response) = &outcome else {
            panic!("{outcome:?}");
        };
        assert_debug_hidden(response);
        assert_debug_hidden(response.body());
        assert_debug_hidden(response.headers());
        // The control: the values were received and stay readable.
        assert_eq!(
            response.headers()[HeaderName::from_static("x-echo")],
            RESPONSE
        );
        if let Some(bytes) = response.body().complete() {
            assert_eq!(bytes, RESPONSE);
        }
        assert_request_carried_markers(&peer).await;
    }
}
