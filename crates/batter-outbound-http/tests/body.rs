//! Responded outcomes: the kept status and the bounded body read.
//!
//! A received head is evidence that survives every body failure, so each
//! test checks the status next to the body result.
#![allow(clippy::unwrap_used, reason = "a panic fails the test")]
#![allow(clippy::missing_assert_message, reason = "test names locate failures")]

#[allow(dead_code, reason = "shared support; this crate uses part of it")]
#[path = "support/peer.rs"]
mod peer;

use batter_outbound_http::{
    Body, BodyFailure, Exchange, RESERVE, Response,
    http::{Method, StatusCode},
};
use peer::{Peer, Step, after, assert_within_reserve, limit, millis, write};
use std::time::Duration;
use tokio::time::Instant;

const LIMIT: usize = 1024;

/// Send one GET with a `LIMIT` body limit and `budget` to a peer running
/// `script`, check that it produced a response to exactly one request, and
/// return it with the caller's deadline. The deadline starts after the client
/// is built, so construction time under load cannot use up the budget.
async fn respond_by(script: Vec<Step>, budget: Duration) -> (Exchange, Instant) {
    let peer = Peer::start(script).await;
    let prepared = peer
        .client()
        .request(Method::GET, ["v1", "effects", "e-1"])
        .prepare()
        .expect("a valid request");
    let deadline = after(budget);
    let outcome = prepared.send(deadline, limit(LIMIT)).await;
    assert!(matches!(outcome, Exchange::Responded(_)), "{outcome:?}");
    assert!(!outcome.proves_unsent());
    peer.single_request().await;
    (outcome, deadline)
}

/// [`respond_by`] with a generous budget.
async fn respond(script: Vec<Step>) -> Exchange {
    respond_by(script, SOON).await.0
}

fn response(outcome: &Exchange) -> &Response {
    match outcome {
        Exchange::Responded(response) => response,
        other => panic!("expected a response, got {other:?}"),
    }
}

const SOON: Duration = Duration::from_secs(2);

/// The chunk that ends a chunked body.
const LAST_CHUNK: &[u8] = b"0\r\n\r\n";

/// A chunked response head and `chunks`, without the last chunk.
fn chunked(chunks: &[&[u8]]) -> Vec<u8> {
    let mut bytes = b"HTTP/1.1 200 OK\r\ntransfer-encoding: chunked\r\n\r\n".to_vec();
    for chunk in chunks {
        bytes.extend_from_slice(format!("{:x}\r\n", chunk.len()).as_bytes());
        bytes.extend_from_slice(chunk);
        bytes.extend_from_slice(b"\r\n");
    }
    bytes
}

#[tokio::test]
async fn complete_response_keeps_status_sensitive_headers_and_body() {
    let script = vec![
        Step::Read,
        write(
            "HTTP/1.1 201 Created\r\ncontent-length: 5\r\nx-request-ref: ref-1\r\n\
             set-cookie: session=s-1\r\n\r\nhello",
        ),
    ];
    let outcome = respond(script).await;
    assert_eq!(outcome.label(), "responded");
    let Exchange::Responded(response) = outcome else {
        unreachable!("checked by respond");
    };
    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(response.headers()["x-request-ref"], "ref-1");
    assert_eq!(response.headers()["set-cookie"], "session=s-1");
    assert!(
        response
            .headers()
            .values()
            .all(|value| value.is_sensitive())
    );
    assert_eq!(
        response.body().complete().map(|b| &b[..]),
        Some(&b"hello"[..])
    );
    let (status, headers, body) = response.into_parts();
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(headers.len(), 3);
    assert!(matches!(body, Body::Complete(bytes) if bytes == "hello"));
}

/// `Bytes` handles of a complete body stay readable after everything else from
/// the exchange is gone.
#[tokio::test]
async fn complete_body_clones_and_slices_outlive_the_exchange() {
    let outcome = respond(vec![
        Step::Read,
        write("HTTP/1.1 200 OK\r\ncontent-length: 11\r\n\r\nhello world"),
    ])
    .await;
    let Exchange::Responded(response) = outcome else {
        unreachable!("checked by respond");
    };
    let (_, _, Body::Complete(bytes)) = response.into_parts() else {
        panic!("expected a complete body");
    };
    let clone = bytes.clone();
    let slice = bytes.slice(6..);
    drop(bytes);
    assert_eq!(clone, "hello world");
    drop(clone);
    assert_eq!(slice, "world");
}

/// Every position has a distinct neighbor, so a copy misplaced when the buffer
/// grows cannot go unnoticed.
#[tokio::test]
async fn chunked_body_growing_across_allocations_keeps_its_exact_bytes() {
    let body: Vec<u8> = (0..LIMIT).map(|index| (index % 251) as u8).collect();
    let chunks: Vec<&[u8]> = body.chunks(97).collect();
    let script = vec![
        Step::Read,
        write([chunked(&chunks), LAST_CHUNK.to_vec()].concat()),
    ];
    let outcome = respond(script).await;
    assert_eq!(
        response(&outcome).body().complete().map(|b| &b[..]),
        Some(&body[..])
    );
}

#[tokio::test]
async fn bodyless_status_is_complete_and_empty() {
    let outcome = respond(vec![Step::Read, write("HTTP/1.1 204 No Content\r\n\r\n")]).await;
    let response = response(&outcome);
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert!(matches!(response.body(), Body::Complete(bytes) if bytes.is_empty()));
}

#[tokio::test]
async fn stalled_body_keeps_the_status_and_one_deadline_bounds_head_and_body() {
    // The head arrives late; a fresh body timer would end far past the deadline.
    let script = vec![
        Step::Read,
        Step::Pause(millis(250)),
        write("HTTP/1.1 202 Accepted\r\ncontent-length: 10\r\nx-request-ref: ref-2\r\n\r\npart"),
        Step::Hold,
    ];
    let (outcome, deadline) = respond_by(script, millis(700)).await;
    assert_within_reserve(deadline, Instant::now());
    assert_eq!(outcome.label(), "responded.body_deadline");
    let response = response(&outcome);
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    assert_eq!(response.headers()["x-request-ref"], "ref-2");
    assert!(matches!(
        response.body(),
        Body::Failed(BodyFailure::Deadline)
    ));
}

/// The exchange is not polled between the request reaching the peer and the
/// final await, so the head, the end of its empty body and the timer are all
/// ready at that poll; the ready results win.
#[tokio::test(flavor = "current_thread")]
async fn head_ready_with_the_expired_timer_is_kept() {
    let peer = Peer::start([Step::Read, write("HTTP/1.1 204 No Content\r\n\r\n")]).await;
    let prepared = peer
        .client()
        .request(Method::GET, ["v1"])
        .prepare()
        .unwrap();
    let deadline = after(millis(800));
    let exchange = prepared.send(deadline, limit(LIMIT));
    tokio::pin!(exchange);
    tokio::select! {
        biased;
        () = peer.wait_requests(1) => {}
        outcome = &mut exchange => panic!("finished before the request arrived: {outcome:?}"),
    }
    peer.wait_writes(1).await;
    // Keep the exchange unpolled while the response reaches the socket.
    tokio::time::sleep(millis(100)).await;
    tokio::time::sleep_until(deadline).await;
    let outcome = exchange.await;
    assert_eq!(outcome.label(), "responded", "{outcome:?}");
    assert_eq!(response(&outcome).status(), StatusCode::NO_CONTENT);
    peer.single_request().await;
}

#[tokio::test]
async fn short_body_then_close_is_a_transport_failure() {
    let script = vec![
        Step::Read,
        write("HTTP/1.1 200 OK\r\ncontent-length: 10\r\n\r\npart"),
    ];
    let outcome = respond(script).await;
    assert_eq!(outcome.label(), "responded.body_transport");
    let response = response(&outcome);
    assert_eq!(response.status(), StatusCode::OK);
    assert!(matches!(
        response.body(),
        Body::Failed(BodyFailure::Transport)
    ));
}

#[tokio::test]
async fn unterminated_chunked_body_then_close_is_a_transport_failure() {
    let outcome = respond(vec![Step::Read, write(chunked(&[b"part"]))]).await;
    let response = response(&outcome);
    assert!(matches!(
        response.body(),
        Body::Failed(BodyFailure::Transport)
    ));
}

#[tokio::test]
async fn declared_length_above_the_limit_is_too_large_without_reading() {
    // The peer never sends the declared body: reading it would end at the deadline.
    let script = vec![
        Step::Read,
        write(format!(
            "HTTP/1.1 200 OK\r\ncontent-length: {}\r\n\r\n",
            LIMIT + 1
        )),
        Step::Hold,
    ];
    let (outcome, deadline) = respond_by(script, Duration::from_secs(3)).await;
    assert!(Instant::now() < deadline - RESERVE - Duration::from_secs(1));
    assert_eq!(outcome.label(), "responded.body_too_large");
    let response = response(&outcome);
    assert_eq!(response.status(), StatusCode::OK);
    assert!(matches!(response.body(), Body::TooLarge));
}

#[tokio::test]
async fn streamed_body_above_the_limit_stops_at_the_exceeding_chunk() {
    // No terminating chunk follows: only an incremental check returns early.
    let full = vec![b'a'; LIMIT];
    let script = vec![Step::Read, write(chunked(&[&full, b"b"])), Step::Hold];
    let (outcome, deadline) = respond_by(script, Duration::from_secs(3)).await;
    assert!(Instant::now() < deadline - RESERVE - Duration::from_secs(1));
    let response = response(&outcome);
    assert_eq!(response.status(), StatusCode::OK);
    assert!(matches!(response.body(), Body::TooLarge));
}

#[tokio::test]
async fn body_exactly_at_the_limit_is_complete() {
    let full = vec![b'a'; LIMIT];
    let mut declared = format!("HTTP/1.1 200 OK\r\ncontent-length: {LIMIT}\r\n\r\n").into_bytes();
    declared.extend_from_slice(&full);
    let (first, second) = full.split_at(LIMIT / 4);
    for script in [
        vec![Step::Read, Step::Write(declared)],
        vec![
            Step::Read,
            write([chunked(&[first, second]), LAST_CHUNK.to_vec()].concat()),
        ],
    ] {
        let outcome = respond(script).await;
        let body = response(&outcome).body().complete();
        assert_eq!(body.map(|b| &b[..]), Some(&full[..]));
    }
}

#[tokio::test]
async fn head_response_length_is_not_a_body() {
    let peer = Peer::start([
        Step::Read,
        write(format!(
            "HTTP/1.1 200 OK\r\ncontent-length: {}\r\n\r\n",
            LIMIT * 8
        )),
    ])
    .await;
    let prepared = peer
        .client()
        .request(Method::HEAD, ["v1"])
        .prepare()
        .unwrap();
    let outcome = prepared.send(after(SOON), limit(LIMIT)).await;
    let response = response(&outcome);
    assert!(matches!(response.body(), Body::Complete(bytes) if bytes.is_empty()));
    assert_eq!(
        peer.single_request().await.request_line(),
        "HEAD /v1 HTTP/1.1"
    );
}

/// Without `Content-Length` or chunked framing the body ends when the peer
/// closes the connection, so a truncated body is indistinguishable from a
/// whole one: the documented limit of `Body::Complete`.
#[tokio::test]
async fn an_undelimited_body_ends_at_connection_close() {
    let script = vec![
        Step::Read,
        write("HTTP/1.1 200 OK\r\nconnection: close\r\n\r\npart"),
    ];
    let outcome = respond(script).await;
    assert_eq!(outcome.label(), "responded");
    assert_eq!(
        response(&outcome).body().complete().map(|b| &b[..]),
        Some(&b"part"[..])
    );
}

/// These bytes are not gzip, so a decoding client would fail. Dev-only reqwest
/// compiles gzip into this build; the owned transport still returns raw bytes.
#[tokio::test]
async fn encoded_body_is_returned_as_received_bytes() {
    let script = vec![
        Step::Read,
        write("HTTP/1.1 200 OK\r\ncontent-encoding: gzip\r\ncontent-length: 8\r\n\r\nnot-gzip"),
    ];
    let outcome = respond(script).await;
    let response = response(&outcome);
    assert_eq!(response.headers()["content-encoding"], "gzip");
    assert_eq!(
        response.body().complete().map(|b| &b[..]),
        Some(&b"not-gzip"[..])
    );
}

/// A readable head and nonempty body are retained after a polling stall, even
/// when the deadline has expired. Reading must not enable any new writes.
#[tokio::test(flavor = "current_thread")]
async fn ready_nonempty_body_is_kept_after_the_deadline() {
    let peer = Peer::start([
        Step::Read,
        write("HTTP/1.1 200 OK\r\ncontent-length: 5\r\n\r\nhello"),
    ])
    .await;
    let prepared = peer
        .client()
        .request(Method::GET, ["v1"])
        .prepare()
        .unwrap();
    let deadline = after(millis(800));
    let exchange = prepared.send(deadline, limit(LIMIT));
    tokio::pin!(exchange);
    tokio::select! {
        biased;
        () = peer.wait_requests(1) => {}
        outcome = &mut exchange => panic!("finished before request arrived: {outcome:?}"),
    }
    peer.wait_writes(1).await;
    tokio::time::sleep_until(deadline).await;
    let outcome = exchange.await;
    assert_eq!(outcome.label(), "responded", "{outcome:?}");
    assert_eq!(response(&outcome).body().complete().unwrap(), "hello");
    peer.single_request().await;
}
