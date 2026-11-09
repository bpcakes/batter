//! Response-body storage through the production send path: real hyper HTTP/1
//! over controlled I/O with a paused clock. Each allocation's destructor records
//! what it held and whether it read as zero while still live.
#![allow(clippy::missing_assert_message, reason = "test names locate failures")]

use super::buffer::probe;
use super::*;
use http::StatusCode;
use std::future::pending;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, DuplexStream, duplex},
    task::JoinHandle,
    time::sleep,
};

const LIMIT: usize = 1024;
const BUDGET: Duration = Duration::from_secs(10);
const PART: &[u8] = b"sensitive-part";
const CHUNKED_HEAD: &str = "HTTP/1.1 200 OK\r\ntransfer-encoding: chunked\r\n\r\n";

/// What the peer does after its last write.
#[derive(Clone, Copy)]
enum Then {
    /// Close its sending side, so the client reads the end of the stream.
    Close,
    /// Keep the connection open until the client drops it.
    Hold,
}

/// Answer the one request with `writes`, letting the client read each one
/// before the next, and return every byte the client ever sent.
fn serve(mut io: DuplexStream, writes: Vec<Vec<u8>>, then: Then) -> JoinHandle<Vec<u8>> {
    tokio::spawn(async move {
        let mut received = Vec::new();
        let mut buffer = [0; 1024];
        while !received.windows(4).any(|end| end == b"\r\n\r\n") {
            let read = io.read(&mut buffer).await.unwrap();
            assert_ne!(read, 0, "the client closed before its request ended");
            received.extend_from_slice(&buffer[..read]);
        }
        for bytes in writes {
            io.write_all(&bytes).await.unwrap();
            // The paused clock advances only after the exchange has gone idle.
            sleep(Duration::from_millis(1)).await;
        }
        if matches!(then, Then::Close) {
            io.shutdown().await.unwrap();
        }
        io.read_to_end(&mut received).await.unwrap();
        received
    })
}

/// One exchange through the production `send_on`, under a `BUDGET` deadline.
async fn exchange<C>(io: DuplexStream, limit: usize, cancelled: C) -> Exchange
where
    C: Future<Output = ()>,
{
    let connected = http1::handshake(TokioIo::new(io)).await.unwrap();
    let request = http::Request::builder()
        .uri("/effect")
        .header("host", "localhost")
        .body(Full::new(Bytes::new()))
        .unwrap();
    let gate = Gate {
        deadline: Instant::now() + BUDGET,
        is_cancelled: || false,
    };
    let timer = sleep_until(gate.deadline);
    tokio::pin!(timer, cancelled);
    let limit = ResponseLimit::new(limit).unwrap();
    send_on(connected, request, &gate, limit, timer, cancelled).await
}

/// Serve `writes` to one exchange. Returns its outcome and what the peer read.
async fn outcome<C>(
    writes: Vec<Vec<u8>>,
    then: Then,
    limit: usize,
    cancelled: C,
) -> (Exchange, Vec<u8>)
where
    C: Future<Output = ()>,
{
    let (client, peer) = duplex(64 * 1024);
    let peer = serve(peer, writes, then);
    let outcome = exchange(client, limit, cancelled).await;
    (outcome, peer.await.unwrap())
}

fn assert_one_request(received: &[u8]) {
    let received = String::from_utf8_lossy(received);
    assert_eq!(
        received.matches("GET /effect HTTP/1.1\r\n").count(),
        1,
        "{received}"
    );
}

/// The status survives the body result, and exactly one request was sent.
fn assert_responded(outcome: &Exchange, label: &str, status: StatusCode, received: &[u8]) {
    assert_eq!(outcome.label(), label, "{outcome:?}");
    let Exchange::Responded(response) = outcome else {
        unreachable!("every expected label is a response");
    };
    assert_eq!(response.status(), status);
    assert_one_request(received);
}

fn with_length(declared: usize, body: &[u8]) -> Vec<u8> {
    let head = format!("HTTP/1.1 202 Accepted\r\ncontent-length: {declared}\r\n\r\n");
    [head.as_bytes(), body].concat()
}

fn chunk(data: &[u8]) -> Vec<u8> {
    [format!("{:x}\r\n", data.len()).as_bytes(), data, b"\r\n"].concat()
}

fn chunked(data: &[u8]) -> Vec<u8> {
    [CHUNKED_HEAD.as_bytes(), &chunk(data)].concat()
}

#[tokio::test(start_paused = true)]
async fn a_complete_body_is_wiped_only_when_its_last_view_is_released() {
    probe::take();
    let body = b"sensitive-body";
    let writes = vec![with_length(body.len(), body)];
    let (outcome, received) = outcome(writes, Then::Hold, LIMIT, pending()).await;
    assert_responded(&outcome, "responded", StatusCode::ACCEPTED, &received);
    let Exchange::Responded(response) = outcome else {
        unreachable!("checked above");
    };
    let (_, _, Body::Complete(bytes)) = response.into_parts() else {
        panic!("expected a complete body");
    };
    let clone = bytes.clone();
    let slice = bytes.slice(10..);
    drop(bytes);
    drop(clone);
    assert!(probe::take().is_empty(), "a remaining view keeps the owner");
    assert_eq!(slice, "body");
    drop(slice);
    // The allocation is the declared length, not the response limit.
    assert_eq!(probe::take(), [probe::wiped(body, body.len())]);
}

#[tokio::test(start_paused = true)]
async fn growth_wipes_each_allocation_it_replaces() {
    probe::take();
    let writes = vec![chunked(b"abcd"), chunk(b"efg"), b"0\r\n\r\n".to_vec()];
    let (outcome, received) = outcome(writes, Then::Hold, LIMIT, pending()).await;
    assert_responded(&outcome, "responded", StatusCode::OK, &received);
    assert_eq!(probe::take(), [probe::wiped(b"abcd", 4)]);
    let Exchange::Responded(response) = outcome else {
        unreachable!("checked above");
    };
    assert_eq!(response.body().complete().unwrap(), "abcdefg");
    drop(response);
    assert_eq!(probe::take(), [probe::wiped(b"abcdefg", 8)]);
}

#[tokio::test(start_paused = true)]
async fn a_streamed_size_limit_failure_wipes_the_accumulated_bytes() {
    probe::take();
    let writes = vec![chunked(PART), chunk(&[b'x'; LIMIT])];
    let (outcome, received) = outcome(writes, Then::Hold, LIMIT, pending()).await;
    assert_responded(
        &outcome,
        "responded.body_too_large",
        StatusCode::OK,
        &received,
    );
    assert_eq!(probe::take(), [probe::wiped(PART, PART.len())]);
}

#[tokio::test(start_paused = true)]
async fn truncated_bodies_wipe_the_accumulated_bytes() {
    let declared = PART.len() + 10;
    for (write, status, allocated) in [
        (with_length(declared, PART), StatusCode::ACCEPTED, declared),
        (chunked(PART), StatusCode::OK, PART.len()),
    ] {
        probe::take();
        let (outcome, received) = outcome(vec![write], Then::Close, LIMIT, pending()).await;
        assert_responded(&outcome, "responded.body_transport", status, &received);
        assert_eq!(probe::take(), [probe::wiped(PART, allocated)]);
    }
}

#[tokio::test(start_paused = true)]
async fn deadline_expiry_during_the_body_wipes_the_accumulated_bytes() {
    probe::take();
    let declared = PART.len() + 10;
    let writes = vec![with_length(declared, PART)];
    let (outcome, received) = outcome(writes, Then::Hold, LIMIT, pending()).await;
    assert_responded(
        &outcome,
        "responded.body_deadline",
        StatusCode::ACCEPTED,
        &received,
    );
    assert_eq!(probe::take(), [probe::wiped(PART, declared)]);
}

#[tokio::test(start_paused = true)]
async fn cancellation_during_the_body_wipes_the_accumulated_bytes() {
    probe::take();
    let declared = PART.len() + 10;
    let writes = vec![with_length(declared, PART)];
    let cancelled = sleep(BUDGET / 2);
    let (outcome, received) = outcome(writes, Then::Hold, LIMIT, cancelled).await;
    assert_responded(
        &outcome,
        "responded.body_cancelled",
        StatusCode::ACCEPTED,
        &received,
    );
    assert_eq!(probe::take(), [probe::wiped(PART, declared)]);
}

#[tokio::test(start_paused = true)]
async fn dropping_the_exchange_after_partial_accumulation_wipes_the_bytes() {
    probe::take();
    let declared = PART.len() + 10;
    let (client, peer) = duplex(64 * 1024);
    let peer = serve(peer, vec![with_length(declared, PART)], Then::Hold);
    let mut in_flight = Box::pin(exchange(client, LIMIT, pending()));
    tokio::select! {
        biased;
        outcome = &mut in_flight => panic!("a stalled body completed: {outcome:?}"),
        () = sleep(BUDGET / 2) => {}
    }
    assert!(probe::take().is_empty(), "the body is still being read");
    drop(in_flight);
    assert_eq!(probe::take(), [probe::wiped(PART, declared)]);
    assert_one_request(&peer.await.unwrap());
}

/// A peer's declared length alone cannot make the exchange allocate more than
/// the cap, whatever the response limit allows.
#[tokio::test(start_paused = true)]
async fn a_declared_body_preallocates_no_more_than_the_cap() {
    probe::take();
    let limit = 4 * PREALLOCATE_MAX;
    let writes = vec![with_length(2 * PREALLOCATE_MAX, PART)];
    let (outcome, received) = outcome(writes, Then::Hold, limit, pending()).await;
    assert_responded(
        &outcome,
        "responded.body_deadline",
        StatusCode::ACCEPTED,
        &received,
    );
    assert_eq!(probe::take(), [probe::wiped(PART, PREALLOCATE_MAX)]);
}

#[tokio::test(start_paused = true)]
async fn a_declared_length_above_the_limit_allocates_nothing() {
    probe::take();
    let writes = vec![with_length(LIMIT + 1, b"")];
    let (outcome, received) = outcome(writes, Then::Hold, LIMIT, pending()).await;
    assert_responded(
        &outcome,
        "responded.body_too_large",
        StatusCode::ACCEPTED,
        &received,
    );
    assert!(probe::take().is_empty());
}
