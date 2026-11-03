//! One provider mutation through the protected outbound exchange, and one
//! attempt whose connection the operating system refuses. Both stay on
//! loopback: a local fixture stands in for the provider and answers one
//! request, and the second client targets a port that is bound but never
//! listens.
//!
//! The exchange reports what this attempt established about delivery, never
//! what the provider did. Writing the dispatch fence before the send, settling
//! it from the outcome, and reading a status as the provider's receipt are
//! application policy kept at this root. Only `Exchange::label` values are
//! printed.

use batter::{
    BoxError,
    operation::{OperationContext, OperationError, OperationOwner},
    outbound::{
        Body, Client, ConnectTimeout, Exchange, Origin, ResponseLimit,
        http::{HeaderName, HeaderValue, Method, StatusCode},
    },
};
use std::{convert::Infallible, io, net::SocketAddr, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpSocket},
    time::timeout_at,
};

/// The whole budget of one mutation, including recording its outcome.
const BUDGET: Duration = Duration::from_secs(5);
/// Kept free after the exchange for recording what it established.
const RECORD_RESERVE: Duration = Duration::from_millis(500);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
/// The provider's documented receipt is small; a larger body is not one.
const RESPONSE_LIMIT: usize = 16 * 1024;
/// A fixture credential. A real root loads it from its secret store.
const CREDENTIAL: &str = "Bearer example-credential";
/// The fixture's receipt, in the shape this example's provider documents.
const RECEIPT: &[u8] = br#"{"receipt":"example-1"}"#;

/// The application's dispatch fence for one mutation: in a real root, a
/// durable row written before the send, saying that an effect may be in flight.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fence {
    /// This attempt wrote nothing, so it adds no possible delivery. Whether a
    /// later attempt would be a replay still depends on earlier attempts of
    /// this mutation. Only outcomes whose `proves_unsent()` holds release it.
    Released,
    /// The provider may have acted. Reconcile with it before any new attempt.
    Held,
    /// The provider's documented receipt arrived.
    Settled,
}

/// Settle the fence from one exchange. The match over `Exchange` has no
/// wildcard arm, so a variant added later cannot be absorbed silently.
fn settle(exchange: &Exchange) -> Fence {
    match exchange {
        // Exactly the outcomes for which `proves_unsent` holds, and only this
        // package can produce them: releasing the fence cannot hide a provider
        // effect of this attempt.
        Exchange::NotStarted(_) | Exchange::NotDispatched(_) => Fence::Released,
        // Possibly delivered, possibly still arriving: never retried as though
        // nothing was sent.
        Exchange::Unknown(_) => Fence::Held,
        // A status line arrived, possibly from a load balancer or proxy rather
        // than the provider. Treating only this status with a complete body as
        // the receipt is this application's reading of its provider's contract.
        Exchange::Responded(response) => match (response.status(), response.body()) {
            (StatusCode::CREATED, Body::Complete(_receipt)) => Fence::Settled,
            _ => Fence::Held,
        },
    }
}

/// Send one mutation under `context`. The fence is written before the send;
/// the settled fence is recorded afterwards in the reserved finalization phase.
///
/// The record runs after every outcome unless the finalization phase is
/// interrupted: the parent context was cancelled, which also cancels
/// finalization, or its budget expired. It is then `None`: the fence written
/// before the send stays held and the mutation is reconciled later.
async fn mutate(
    client: &Client,
    context: &OperationContext,
) -> Result<(Exchange, Option<Fence>), BoxError> {
    let limit = ResponseLimit::new(RESPONSE_LIMIT)?;
    // Every construction error surfaces here, before anything is sent. The
    // header value is marked sensitive by the draft.
    let request = client
        .request(Method::POST, ["v1", "effects"])
        .header(
            HeaderName::from_static("authorization"),
            HeaderValue::from_static(CREDENTIAL),
        )
        .json(&serde_json::json!({ "effect": "example", "amount": 1 }))
        .prepare()?;
    let phases = context.reserve_finalization(RECORD_RESERVE)?;
    // Write the in-flight fence durably before anything can be sent. If this
    // write fails, nothing is sent.
    phases
        .work()
        .run("example.fence", |_scope| async {
            // Write "an effect may be in flight" to the application's own store
            // here. This example keeps no store.
            Ok::<_, Infallible>(())
        })
        .await?;
    // The exchange classifies its outcome before the work deadline, so the
    // finalization reserve is still free when it returns.
    let exchange = request.send_in(phases.work(), limit).await;
    let fence = settle(&exchange);
    let recorded = match phases
        .finalization()
        .run("example.record", |_scope| async move {
            // Write the fence and `exchange.label()` to the application's own
            // store here.
            Ok::<_, Infallible>(fence)
        })
        .await
    {
        Ok(fence) => Some(fence),
        // Interrupted: the record step did not run, and the fence written
        // before the send stays held.
        Err(OperationError::Interrupted(_)) => None,
        // A fallible store error must be propagated, never discarded: this arm
        // stops compiling when the record step gains a real error type.
        Err(OperationError::Failed(never)) => match never {},
    };
    Ok((exchange, recorded))
}

/// A local stand-in for the provider: read one request and answer it with
/// `head` and `body`, then close the connection. Returns the request bytes.
async fn answer_once(listener: TcpListener, head: String, body: &[u8]) -> io::Result<Vec<u8>> {
    let (mut stream, _) = listener.accept().await?;
    let request = read_request(&mut stream).await?;
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(body).await?;
    stream.shutdown().await?;
    Ok(request)
}

/// The fixture's documented receipt.
fn receipt_head() -> String {
    format!(
        "HTTP/1.1 201 Created\r\ncontent-type: application/json\r\n\
         content-length: {}\r\nconnection: close\r\n\r\n",
        RECEIPT.len()
    )
}

async fn read_request(stream: &mut tokio::net::TcpStream) -> io::Result<Vec<u8>> {
    let mut request = Vec::new();
    let mut chunk = [0; 1024];
    let mut length = None;
    while length.is_none_or(|total| request.len() < total) {
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        request.extend_from_slice(&chunk[..read]);
        length = length.or_else(|| request_length(&request));
    }
    Ok(request)
}

/// The length of a request whose head has arrived: the head plus the body
/// length it declares.
fn request_length(request: &[u8]) -> Option<usize> {
    let head = request
        .windows(4)
        .position(|window| window == b"\r\n\r\n")?
        + 4;
    let declared = std::str::from_utf8(&request[..head])
        .ok()?
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            if name.eq_ignore_ascii_case("content-length") {
                value.trim().parse::<usize>().ok()
            } else {
                None
            }
        });
    Some(head + declared.unwrap_or(0))
}

/// A port that is bound but never listens, so the operating system refuses
/// every connection to it. Keep the socket alive for the whole attempt: a
/// released port could be reassigned to another listener at once.
fn refusing_port() -> io::Result<(TcpSocket, SocketAddr)> {
    let socket = TcpSocket::new_v4()?;
    socket.bind(SocketAddr::from(([127, 0, 0, 1], 0)))?;
    let address = socket.local_addr()?;
    Ok((socket, address))
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), BoxError> {
    // First statement, unwrapped: prints each panic's location, never its payload.
    std::panic::set_hook(batter::redacting_panic_hook());
    let connect_timeout = ConnectTimeout::new(CONNECT_TIMEOUT)?;

    // Build each client before its budget starts. The fixture gives up at the
    // mutation's own deadline, so neither waits on the other past the budget.
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let client = Client::new(Origin::loopback(listener.local_addr()?)?, connect_timeout)?;
    let context = OperationOwner::new(BUDGET)?.into_context();
    let (received, sent) = tokio::join!(
        timeout_at(
            context.deadline(),
            answer_once(listener, receipt_head(), RECEIPT)
        ),
        mutate(&client, &context),
    );
    let (exchange, _fence) = sent?;
    // A label carries no status, header, body, URL or credential.
    println!("mutation exchange: {}", exchange.label());
    let _request = received??;

    // The operating system refuses the connection before any request byte is
    // written, so the exchange proves this attempt unsent.
    let (_bound, closed) = refusing_port()?;
    let client = Client::new(Origin::loopback(closed)?, connect_timeout)?;
    let context = OperationOwner::new(BUDGET)?.into_context();
    let (unreachable, _fence) = mutate(&client, &context).await?;
    println!("closed-port exchange: {}", unreachable.label());
    Ok(())
}

#[cfg(test)]
#[path = "outbound_exchange/tests.rs"]
mod tests;
