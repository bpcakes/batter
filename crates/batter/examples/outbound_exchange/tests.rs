//! Consumer-contract tests for the outbound example. Each fence decision for an
//! outcome only the package can produce is driven by a real exchange against a
//! test-owned loopback listener or a bound port that never listens; the
//! mapping table adds the conservative `Unknown` values a caller may build by
//! hand. Every client is built before its budget starts.

use super::*;
use batter::outbound_http::{
    ConnectFailure, MINIMUM_ATTEMPT, RESERVE, StartRefusal, UnknownReason,
};
use tokio::{sync::oneshot, time::Instant};

fn client(address: SocketAddr) -> Client {
    let timeout = ConnectTimeout::new(CONNECT_TIMEOUT).unwrap();
    Client::new(Origin::loopback(address).unwrap(), timeout).unwrap()
}

#[tokio::test]
async fn the_fixture_receipt_settles_the_fence_after_one_delivered_request() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client = client(listener.local_addr().unwrap());
    let context = OperationOwner::new(BUDGET).unwrap().into_context();
    let (received, sent) = tokio::join!(
        timeout_at(
            context.deadline(),
            answer_once(listener, receipt_head(), RECEIPT)
        ),
        mutate(&client, &context),
    );
    let (exchange, fence) = sent.unwrap();
    assert_eq!(exchange.label(), "responded");
    assert!(!exchange.proves_unsent());
    assert_eq!(fence, Some(Fence::Settled));
    let Exchange::Responded(response) = &exchange else {
        panic!("{exchange:?}")
    };
    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(
        response.body().complete().map(|bytes| &bytes[..]),
        Some(RECEIPT)
    );

    let request = String::from_utf8(received.unwrap().unwrap()).unwrap();
    let (head, body) = request.split_once("\r\n\r\n").unwrap();
    assert!(head.starts_with("POST /v1/effects HTTP/1.1\r\n"), "{head}");
    let head = head.to_ascii_lowercase();
    assert!(head.contains("\r\nauthorization: bearer example-credential"));
    assert!(head.contains("\r\ncontent-type: application/json"));
    let body: serde_json::Value = serde_json::from_str(body).unwrap();
    assert_eq!(
        body,
        serde_json::json!({ "effect": "example", "amount": 1 })
    );
}

#[tokio::test]
async fn a_receipt_status_without_a_usable_body_keeps_the_fence_held() {
    // The documented status arrived, but its body exceeds the receipt limit,
    // so this application cannot read a receipt: the fence stays held.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client = client(listener.local_addr().unwrap());
    let head = format!(
        "HTTP/1.1 201 Created\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        RESPONSE_LIMIT + 1
    );
    let context = OperationOwner::new(BUDGET).unwrap().into_context();
    let (received, sent) = tokio::join!(
        timeout_at(context.deadline(), answer_once(listener, head, b"")),
        mutate(&client, &context),
    );
    let (exchange, fence) = sent.unwrap();
    assert_eq!(exchange.label(), "responded.body_too_large");
    let Exchange::Responded(response) = &exchange else {
        panic!("{exchange:?}")
    };
    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(fence, Some(Fence::Held));
    received.unwrap().unwrap();
}

#[tokio::test]
async fn a_refused_connection_proves_the_mutation_unsent_and_releases_the_fence() {
    let (_bound, closed) = refusing_port().unwrap();
    let client = client(closed);
    let context = OperationOwner::new(BUDGET).unwrap().into_context();
    let (exchange, fence) = mutate(&client, &context).await.unwrap();
    assert!(
        matches!(&exchange, Exchange::NotDispatched(failure) if failure.failure() == ConnectFailure::Failed),
        "{exchange:?}"
    );
    assert_eq!(exchange.label(), "not_dispatched.connect_failed");
    assert!(exchange.proves_unsent());
    assert_eq!(fence, Some(Fence::Released));
}

#[tokio::test]
async fn a_budget_too_short_to_attempt_is_refused_unsent_and_still_recorded() {
    // The work phase ends RECORD_RESERVE before the deadline, and the exchange
    // needs RESERVE + MINIMUM_ATTEMPT of it. One millisecond short leaves the
    // fence step the most room while the start gate still refuses. Against a
    // refusing port a started exchange would report NotDispatched instead.
    let (_bound, closed) = refusing_port().unwrap();
    let client = client(closed);
    let budget = RECORD_RESERVE + RESERVE + MINIMUM_ATTEMPT - Duration::from_millis(1);
    let context = OperationOwner::new(budget).unwrap().into_context();
    let (exchange, fence) = mutate(&client, &context).await.unwrap();
    assert!(
        matches!(&exchange, Exchange::NotStarted(refusal) if refusal.reason() == StartRefusal::DeadlineTooClose),
        "{exchange:?}"
    );
    assert_eq!(fence, Some(Fence::Released));
}

#[tokio::test]
async fn parent_cancellation_in_flight_holds_the_fence_and_records_nothing() {
    // The fixture reads the request and never answers. Cancelling the parent
    // then cancels the work phase and the finalization phase together.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client = client(listener.local_addr().unwrap());
    let (read, request_read) = oneshot::channel();
    let fixture = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        read_request(&mut stream).await.unwrap();
        let _ = read.send(());
        // Keep the connection open without answering.
        let mut rest = [0; 64];
        while matches!(stream.read(&mut rest).await, Ok(count) if count > 0) {}
    });
    let owner = OperationOwner::new(BUDGET).unwrap();
    let cancel = async {
        request_read.await.unwrap();
        owner.cancel();
    };
    let (sent, ()) = tokio::join!(mutate(&client, owner.context()), cancel);
    let (exchange, fence) = sent.unwrap();
    assert!(
        matches!(exchange, Exchange::Unknown(UnknownReason::Cancelled)),
        "{exchange:?}"
    );
    assert_eq!(fence, None, "a cancelled parent runs no record step");
    assert_eq!(settle(&exchange), Fence::Held);
    fixture.abort();
}

#[tokio::test]
async fn only_outcomes_that_prove_nothing_was_sent_release_the_fence() {
    let (_bound, closed) = refusing_port().unwrap();
    let client = client(closed);
    let request = || {
        client
            .request(Method::POST, ["v1", "effects"])
            .prepare()
            .unwrap()
    };
    let limit = ResponseLimit::new(RESPONSE_LIMIT).unwrap();
    // Real unsent outcomes: no caller can construct them.
    let not_started = request().send(Instant::now(), limit).await;
    let not_dispatched = request()
        .send(Instant::now() + Duration::from_secs(2), limit)
        .await;
    assert_eq!(not_started.label(), "not_started.deadline");
    assert_eq!(not_dispatched.label(), "not_dispatched.connect_failed");
    for exchange in [
        not_started,
        not_dispatched,
        Exchange::Unknown(UnknownReason::Deadline),
        Exchange::Unknown(UnknownReason::Cancelled),
        Exchange::Unknown(UnknownReason::Transport),
    ] {
        let released = settle(&exchange) == Fence::Released;
        assert_eq!(released, exchange.proves_unsent(), "{}", exchange.label());
        assert_eq!(
            settle(&exchange) == Fence::Held,
            matches!(exchange, Exchange::Unknown(_))
        );
    }
}

#[test]
fn a_request_is_complete_once_its_declared_body_arrived() {
    let head = b"POST /v1 HTTP/1.1\r\nContent-Length: 2\r\n\r\n";
    assert_eq!(request_length(b"POST /v1 HTTP/1.1\r\n"), None);
    assert_eq!(request_length(head), Some(head.len() + 2));
    assert_eq!(request_length(b"GET / HTTP/1.1\r\n\r\n"), Some(18));
}
