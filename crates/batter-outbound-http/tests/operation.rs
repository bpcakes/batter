//! `send_in` under an `OperationContext`'s deadline and cancellation.
//!
//! Cancellation before the start proves nothing was sent; cancellation after
//! the start never claims more than the peer's evidence supports. Each test
//! prepares its request before its context's budget starts, so client
//! construction time under load cannot use up the budget.
#![cfg(feature = "operation")]
#![allow(clippy::unwrap_used, reason = "a panic fails the test")]
#![allow(clippy::missing_assert_message, reason = "test names locate failures")]

#[allow(dead_code, reason = "shared support; this crate uses part of it")]
#[path = "support/peer.rs"]
mod peer;

use batter_core::operation::{OperationOwner, RootDeadline};
use batter_outbound_http::{
    Body, BodyFailure, Exchange, MINIMUM_ATTEMPT, PreparedRequest, RESERVE, StartRefusal,
    UnknownReason,
    http::{Method, StatusCode},
};
use peer::{Peer, Step, assert_within_reserve, limit, millis, write};
use std::time::Duration;
use tokio::time::Instant;

fn effect(peer: &Peer) -> PreparedRequest {
    peer.client()
        .request(Method::POST, ["v1", "effects"])
        .json(&serde_json::json!({ "effect": "example" }))
        .prepare()
        .expect("a valid request")
}

fn owner(budget: Duration) -> OperationOwner {
    OperationOwner::new(budget).expect("a valid budget")
}

const HEAD_ONLY: &str = "HTTP/1.1 204 No Content\r\n\r\n";

#[tokio::test]
async fn cancelled_context_does_not_start() {
    let peer = Peer::start([Step::Read, Step::Hold]).await;
    let request = effect(&peer);
    let operation = owner(Duration::from_secs(5));
    operation.cancel();
    let outcome = request.send_in(operation.context(), limit(1024)).await;
    assert!(
        matches!(&outcome, Exchange::NotStarted(refusal) if refusal.reason() == StartRefusal::Cancelled),
        "{outcome:?}"
    );
    assert!(outcome.proves_unsent());
    assert_eq!(outcome.label(), "not_started.cancelled");
    peer.assert_untouched().await;
}

#[tokio::test]
async fn cancellation_takes_precedence_over_an_expired_deadline() {
    let peer = Peer::start([Step::Read, Step::Hold]).await;
    let expired = OperationOwner::at(RootDeadline::at(Instant::now()));
    let outcome = effect(&peer).send_in(expired.context(), limit(1024)).await;
    assert!(
        matches!(&outcome, Exchange::NotStarted(refusal) if refusal.reason() == StartRefusal::DeadlineTooClose),
        "{outcome:?}"
    );
    expired.cancel();
    let outcome = effect(&peer).send_in(expired.context(), limit(1024)).await;
    assert!(
        matches!(&outcome, Exchange::NotStarted(refusal) if refusal.reason() == StartRefusal::Cancelled),
        "{outcome:?}"
    );
    peer.assert_untouched().await;
}

#[tokio::test]
async fn context_deadline_too_close_does_not_start() {
    let peer = Peer::start([Step::Read, Step::Hold]).await;
    let request = effect(&peer);
    let operation = owner(RESERVE + MINIMUM_ATTEMPT - millis(5));
    let outcome = request.send_in(operation.context(), limit(1024)).await;
    assert!(
        matches!(&outcome, Exchange::NotStarted(refusal) if refusal.reason() == StartRefusal::DeadlineTooClose),
        "{outcome:?}"
    );
    peer.assert_untouched().await;
}

#[tokio::test]
async fn cancellation_after_the_request_reached_the_peer_is_unknown() {
    let peer = Peer::start([Step::Read, Step::Hold]).await;
    let request = effect(&peer);
    let operation = owner(Duration::from_secs(5));
    let (outcome, ()) = tokio::join!(request.send_in(operation.context(), limit(1024)), async {
        peer.wait_requests(1).await;
        operation.cancel();
    });
    assert!(
        matches!(outcome, Exchange::Unknown(UnknownReason::Cancelled)),
        "{outcome:?}"
    );
    assert!(!outcome.proves_unsent());
    assert_eq!(outcome.label(), "unknown.cancelled");
    assert!(operation.context().remaining() > Duration::from_secs(3));
    peer.single_request().await;
}

#[tokio::test]
async fn cancellation_while_the_body_stalls_keeps_the_status() {
    let peer = Peer::start([
        Step::Read,
        write("HTTP/1.1 202 Accepted\r\ncontent-length: 10\r\n\r\npart"),
        Step::Hold,
    ])
    .await;
    let request = effect(&peer);
    let operation = owner(Duration::from_secs(5));
    let (outcome, ()) = tokio::join!(request.send_in(operation.context(), limit(1024)), async {
        peer.wait_writes(1).await;
        // Let the head and the first body bytes reach the exchange.
        tokio::time::sleep(millis(300)).await;
        operation.cancel();
    });
    assert_eq!(outcome.label(), "responded.body_cancelled");
    let Exchange::Responded(response) = outcome else {
        panic!("{outcome:?}");
    };
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    assert!(matches!(
        response.body(),
        Body::Failed(BodyFailure::Cancelled)
    ));
    peer.single_request().await;
}

/// The exchange is not polled between the request reaching the peer and the
/// final await, so the head and the cancellation are both ready at that poll;
/// the head wins.
#[tokio::test(flavor = "current_thread")]
async fn head_ready_with_cancellation_is_kept() {
    let peer = Peer::start([Step::Read, write(HEAD_ONLY)]).await;
    let request = effect(&peer);
    let operation = owner(Duration::from_secs(5));
    let exchange = request.send_in(operation.context(), limit(1024));
    tokio::pin!(exchange);
    tokio::select! {
        biased;
        () = peer.wait_requests(1) => {}
        outcome = &mut exchange => panic!("finished before the request arrived: {outcome:?}"),
    }
    peer.wait_writes(1).await;
    tokio::time::sleep(millis(200)).await;
    operation.cancel();
    let outcome = exchange.await;
    assert_eq!(outcome.label(), "responded", "{outcome:?}");
    let Exchange::Responded(response) = outcome else {
        unreachable!("checked by the label");
    };
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    peer.single_request().await;
}

/// Without a head, a cancellation and an expired timer that are both ready
/// at the next poll report the cancellation, like batter-core's precedence.
#[tokio::test(flavor = "current_thread")]
async fn cancellation_ready_with_the_expired_timer_is_reported() {
    let peer = Peer::start([Step::Read, Step::Hold]).await;
    let request = effect(&peer);
    let operation = owner(millis(800));
    let exchange_deadline = operation.context().deadline() - RESERVE;
    let exchange = request.send_in(operation.context(), limit(1024));
    tokio::pin!(exchange);
    tokio::select! {
        biased;
        () = peer.wait_requests(1) => {}
        outcome = &mut exchange => panic!("finished before the request arrived: {outcome:?}"),
    }
    tokio::time::sleep_until(exchange_deadline + millis(20)).await;
    operation.cancel();
    let outcome = exchange.await;
    assert_eq!(outcome.label(), "unknown.cancelled", "{outcome:?}");
    peer.single_request().await;
}

/// During the body as well, a cancellation and an expired timer that are both
/// ready at the next poll report the cancellation. The head arrives with part
/// of a declared body and the rest never comes.
#[tokio::test(flavor = "current_thread")]
async fn cancellation_ready_with_the_expired_timer_during_the_body_is_reported() {
    let script = [
        Step::Read,
        write("HTTP/1.1 200 OK\r\ncontent-length: 10\r\n\r\npart"),
        Step::Hold,
    ];
    let peer = Peer::start(script).await;
    let request = effect(&peer);
    let operation = owner(millis(800));
    let exchange_deadline = operation.context().deadline() - RESERVE;
    let exchange = request.send_in(operation.context(), limit(1024));
    tokio::pin!(exchange);
    tokio::select! {
        biased;
        () = peer.wait_requests(1) => {}
        outcome = &mut exchange => panic!("finished before the request arrived: {outcome:?}"),
    }
    peer.wait_writes(1).await;
    // Let the head arrive, then poll once so the exchange starts reading the
    // body before both the timer and the cancellation become ready.
    tokio::time::sleep(millis(100)).await;
    tokio::select! {
        biased;
        outcome = &mut exchange => panic!("finished before the timer: {outcome:?}"),
        () = tokio::time::sleep(millis(10)) => {}
    }
    tokio::time::sleep_until(exchange_deadline + millis(20)).await;
    operation.cancel();
    let outcome = exchange.await;
    assert_eq!(outcome.label(), "responded.body_cancelled", "{outcome:?}");
    peer.single_request().await;
}

#[tokio::test]
async fn context_deadline_bounds_the_exchange_and_returns_before_it() {
    let peer = Peer::start([Step::Read, Step::Hold]).await;
    let request = effect(&peer);
    let operation = owner(millis(600));
    let outcome = request.send_in(operation.context(), limit(1024)).await;
    assert_within_reserve(operation.context().deadline(), Instant::now());
    // The classification arrives before the context expires, so an outer timer
    // at the same deadline would not discard it. RESERVE is not a recording
    // budget: recording belongs in a separately reserved finalization phase.
    assert_eq!(operation.context().check(), Ok(()));
    assert!(
        matches!(outcome, Exchange::Unknown(UnknownReason::Deadline)),
        "{outcome:?}"
    );
    peer.single_request().await;
}

#[tokio::test]
async fn context_deadline_bounds_a_stalled_body() {
    let peer = Peer::start([
        Step::Read,
        write("HTTP/1.1 200 OK\r\ncontent-length: 10\r\n\r\npart"),
        Step::Hold,
    ])
    .await;
    let request = effect(&peer);
    let operation = owner(millis(600));
    let outcome = request.send_in(operation.context(), limit(1024)).await;
    assert_within_reserve(operation.context().deadline(), Instant::now());
    let Exchange::Responded(response) = outcome else {
        panic!("{outcome:?}");
    };
    assert_eq!(response.status(), StatusCode::OK);
    assert!(matches!(
        response.body(),
        Body::Failed(BodyFailure::Deadline)
    ));
}

#[tokio::test]
async fn child_context_cancelled_through_its_parent_is_unknown() {
    let peer = Peer::start([Step::Read, Step::Hold]).await;
    let request = effect(&peer);
    let parent = owner(Duration::from_secs(5));
    let child = parent
        .context()
        .child(Duration::from_secs(2))
        .expect("a valid child budget");
    let (outcome, ()) = tokio::join!(request.send_in(child.context(), limit(1024)), async {
        peer.wait_requests(1).await;
        parent.cancel();
    });
    assert!(
        matches!(outcome, Exchange::Unknown(UnknownReason::Cancelled)),
        "{outcome:?}"
    );
    peer.single_request().await;
}
