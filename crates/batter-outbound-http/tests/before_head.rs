//! Outcomes before a response head: refusals, connect failures and unknowns.
//!
//! Every request goes to a loopback fault peer that counts connections and
//! complete requests, so each outcome is checked against what reached it.

#[allow(dead_code)]
#[path = "support/peer.rs"]
mod peer;

use batter_outbound_http::{
    Client, ConnectFailure, Exchange, MINIMUM_ATTEMPT, PreparedRequest, RESERVE, StartRefusal,
    UnknownReason, http::Method,
};
use peer::{Peer, RefusingPort, Step, after, assert_within_reserve, limit, millis, write};
use std::time::Duration;
use tokio::time::Instant;

fn effect(client: &Client) -> PreparedRequest {
    client
        .request(Method::POST, ["v1", "effects"])
        .json(&serde_json::json!({ "effect": "example" }))
        .prepare()
        .expect("a valid request")
}

async fn send(prepared: PreparedRequest, deadline: Instant) -> Exchange {
    prepared.send(deadline, limit(1024)).await
}

#[tokio::test]
async fn passed_deadline_does_not_start() {
    let peer = Peer::start([Step::Read, Step::Hold]).await;
    let passed = Instant::now()
        .checked_sub(Duration::from_secs(1))
        .expect("an earlier instant");
    let outcome = send(effect(&peer.client()), passed).await;
    assert!(
        matches!(&outcome, Exchange::NotStarted(refusal) if refusal.reason() == StartRefusal::DeadlineTooClose),
        "{outcome:?}"
    );
    assert!(outcome.proves_unsent());
    assert_eq!(outcome.label(), "not_started.deadline");
    peer.assert_untouched().await;
}

#[tokio::test]
async fn deadline_inside_reserve_and_minimum_attempt_does_not_start() {
    let peer = Peer::start([Step::Read, Step::Hold]).await;
    let window = RESERVE + MINIMUM_ATTEMPT;
    for budget in [Duration::ZERO, RESERVE, window - millis(5)] {
        let outcome = send(effect(&peer.client()), after(budget)).await;
        assert!(
            matches!(&outcome, Exchange::NotStarted(refusal) if refusal.reason() == StartRefusal::DeadlineTooClose),
            "{budget:?}: {outcome:?}"
        );
        assert!(outcome.proves_unsent());
    }
    peer.assert_untouched().await;
}

#[tokio::test]
async fn deadline_beyond_the_refusal_window_starts_one_request() {
    let peer = Peer::start([Step::Read, Step::Hold]).await;
    let prepared = effect(&peer.client());
    let outcome = send(prepared, after(RESERVE + MINIMUM_ATTEMPT + millis(250))).await;
    assert!(
        matches!(outcome, Exchange::Unknown(UnknownReason::Deadline)),
        "{outcome:?}"
    );
    assert_eq!(
        peer.single_request().await.request_line(),
        "POST /v1/effects HTTP/1.1"
    );
}

#[tokio::test]
async fn refused_connection_is_not_dispatched() {
    let refusing = RefusingPort::bind();
    let outcome = send(effect(&refusing.client()), after(Duration::from_secs(2))).await;
    assert!(
        matches!(&outcome, Exchange::NotDispatched(failure) if failure.failure() == ConnectFailure::Failed),
        "{outcome:?}"
    );
    assert!(outcome.proves_unsent());
    assert_eq!(outcome.label(), "not_dispatched.connect_failed");
}

#[tokio::test]
async fn failed_tls_handshake_is_not_dispatched_and_sends_no_http() {
    let peer = Peer::start([
        Step::Sniff,
        write("HTTP/1.1 400 Bad Request\r\ncontent-length: 0\r\n\r\n"),
    ])
    .await;
    let client = peer.tls_client(Duration::from_secs(2));
    let outcome = send(effect(&client), after(Duration::from_secs(2))).await;
    assert!(
        matches!(&outcome, Exchange::NotDispatched(failure) if failure.failure() == ConnectFailure::Failed),
        "{outcome:?}"
    );
    assert!(outcome.proves_unsent());
    assert_client_hello_only(&peer).await;
}

/// The connection carried a TLS handshake record and no HTTP request.
async fn assert_client_hello_only(peer: &Peer) {
    peer.wait_sniffed(1).await;
    assert_eq!(peer.connections(), 1);
    let sniffed = peer.sniffed();
    assert_eq!(sniffed.len(), 1);
    assert_eq!(
        sniffed[0].first(),
        Some(&0x16),
        "not a TLS handshake record"
    );
    let text = String::from_utf8_lossy(&sniffed[0]);
    assert!(!text.contains("HTTP/1.1") && !text.contains("/v1/effects"));
}

#[tokio::test]
async fn stalled_tls_handshake_times_out_as_not_dispatched() {
    let peer = Peer::start([Step::Sniff, Step::Hold]).await;
    let connect = millis(300);
    let prepared = effect(&peer.tls_client(connect));
    let started = Instant::now();
    let deadline = after(Duration::from_secs(3));
    let outcome = send(prepared, deadline).await;
    let returned = Instant::now();
    assert!(
        matches!(&outcome, Exchange::NotDispatched(failure) if failure.failure() == ConnectFailure::TimedOut),
        "{outcome:?}"
    );
    assert!(outcome.proves_unsent());
    assert_eq!(outcome.label(), "not_dispatched.connect_timeout");
    assert!(returned - started >= connect);
    assert!(returned < deadline - RESERVE);
    assert_client_hello_only(&peer).await;
}

#[tokio::test]
async fn deadline_expiring_while_connecting_is_unknown() {
    let peer = Peer::start([Step::Sniff, Step::Hold]).await;
    let prepared = effect(&peer.tls_client(Duration::from_secs(10)));
    let deadline = after(millis(600));
    let outcome = send(prepared, deadline).await;
    assert_within_reserve(deadline, Instant::now());
    // Conservative: the exchange cannot tell which phase its timer stopped.
    assert!(
        matches!(outcome, Exchange::Unknown(UnknownReason::Deadline)),
        "{outcome:?}"
    );
    assert!(!outcome.proves_unsent());
    assert_client_hello_only(&peer).await;
}

#[tokio::test]
async fn close_before_reading_is_unknown_on_one_connection() {
    let peer = Peer::start([]).await;
    let outcome = send(effect(&peer.client()), after(Duration::from_secs(2))).await;
    assert!(
        matches!(outcome, Exchange::Unknown(UnknownReason::Transport)),
        "{outcome:?}"
    );
    assert!(!outcome.proves_unsent());
    assert_eq!(outcome.label(), "unknown.transport");
    assert_eq!(peer.connections(), 1, "the request was replayed");
}

#[tokio::test]
async fn close_after_the_full_request_is_unknown() {
    let peer = Peer::start([Step::Read]).await;
    let outcome = send(effect(&peer.client()), after(Duration::from_secs(2))).await;
    assert!(
        matches!(outcome, Exchange::Unknown(UnknownReason::Transport)),
        "{outcome:?}"
    );
    assert!(!outcome.proves_unsent());
    assert_eq!(
        peer.single_request().await.body(),
        br#"{"effect":"example"}"#
    );
}

#[tokio::test]
async fn reset_after_the_full_request_is_unknown() {
    let peer = Peer::start([Step::Read, Step::Reset]).await;
    let outcome = send(effect(&peer.client()), after(Duration::from_secs(2))).await;
    assert!(
        matches!(outcome, Exchange::Unknown(UnknownReason::Transport)),
        "{outcome:?}"
    );
    assert_eq!(
        peer.single_request().await.body(),
        br#"{"effect":"example"}"#
    );
}

#[tokio::test]
async fn malformed_or_truncated_head_is_unknown() {
    for head in [
        &b"NOT-HTTP 200\r\n\r\n"[..],
        b"HTTP/1.1 200 OK\r\ncontent-len",
    ] {
        let peer = Peer::start([Step::Read, write(head)]).await;
        let outcome = send(effect(&peer.client()), after(Duration::from_secs(2))).await;
        assert!(
            matches!(outcome, Exchange::Unknown(UnknownReason::Transport)),
            "{outcome:?}"
        );
        peer.single_request().await;
    }
}

#[tokio::test]
async fn silent_peer_is_unknown_within_the_reserve() {
    let peer = Peer::start([Step::Read, Step::Hold]).await;
    let prepared = effect(&peer.client());
    let deadline = after(millis(600));
    let outcome = send(prepared, deadline).await;
    assert_within_reserve(deadline, Instant::now());
    assert!(
        matches!(outcome, Exchange::Unknown(UnknownReason::Deadline)),
        "{outcome:?}"
    );
    assert!(!outcome.proves_unsent());
    assert_eq!(outcome.label(), "unknown.deadline");
    peer.single_request().await;
}
