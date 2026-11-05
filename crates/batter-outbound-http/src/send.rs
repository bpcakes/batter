//! The single send path and its phase-owned transport classification.

use crate::{
    client::Client,
    exchange::{
        Body, BodyFailure, ConnectFailure, Exchange, NotDispatched, NotStarted, Response,
        StartRefusal, UnknownReason,
    },
    limits::ResponseLimit,
    transport::{self, Gate},
};
use bytes::Bytes;
use http::HeaderMap;
use http_body_util::{BodyExt, Full};
use hyper::{body::Incoming, client::conn::http1};
use hyper_util::rt::TokioIo;
use std::{
    future::{Future, poll_fn},
    pin::Pin,
    task::Poll,
    time::Duration,
};
use tokio::time::{Instant, Sleep, sleep_until};

/// Time kept free at the end of the caller's deadline, apart from scheduling
/// delay and blocking polls such as macOS certificate verification. This lets
/// classification precede an outer timer. Reserve recording time separately.
pub const RESERVE: Duration = Duration::from_millis(50);

/// The shortest window in which connection establishment will begin.
pub const MINIMUM_ATTEMPT: Duration = Duration::from_millis(50);

const PREALLOCATE_MAX: usize = 1024 * 1024;

pub(crate) async fn run<C, F>(
    client: Client,
    request: http::Request<Full<Bytes>>,
    deadline: Instant,
    limit: ResponseLimit,
    cancelled: C,
    is_cancelled: F,
) -> Exchange
where
    C: Future<Output = ()>,
    F: Fn() -> bool + Sync,
{
    let Some(exchange_deadline) = deadline.checked_sub(RESERVE) else {
        return not_started(StartRefusal::DeadlineTooClose);
    };
    let gate = Gate {
        deadline: exchange_deadline,
        is_cancelled,
    };
    if let Some(reason) = gate.stopped() {
        return not_started(reason);
    }
    if exchange_deadline.saturating_duration_since(Instant::now()) < MINIMUM_ATTEMPT {
        return not_started(StartRefusal::DeadlineTooClose);
    }
    let timer = sleep_until(exchange_deadline);
    let connect_deadline = Instant::now() + client.connect_timeout;
    tokio::pin!(timer, cancelled);
    let connect = async {
        let io = transport::connect(&client, &gate, connect_deadline).await?;
        http1::handshake(TokioIo::new(io))
            .await
            .map_err(std::io::Error::other)
    };
    let connected = await_setup(
        connect,
        &gate,
        connect_deadline,
        timer.as_mut(),
        cancelled.as_mut(),
    )
    .await;
    let connected = match connected {
        Ok(connected) => connected,
        Err(failure) => return failure.into_exchange(),
    };
    send_on(connected, request, &gate, limit, timer, cancelled).await
}

/// Setup cannot produce a response or post-handoff uncertainty.
enum SetupFailure {
    Stopped(StartRefusal),
    Connection(ConnectFailure),
}

impl SetupFailure {
    fn into_exchange(self) -> Exchange {
        match self {
            Self::Stopped(reason) => not_started(reason),
            Self::Connection(failure) => not_dispatched(failure),
        }
    }
}

/// Wait for the owned setup, retaining direct clock checks after its last poll.
async fn await_setup<T, S, C, F>(
    connect: S,
    gate: &Gate<F>,
    connect_deadline: Instant,
    mut timer: Pin<&mut Sleep>,
    mut cancelled: Pin<&mut C>,
) -> Result<T, SetupFailure>
where
    S: Future<Output = std::io::Result<T>>,
    C: Future<Output = ()>,
    F: Fn() -> bool,
{
    let connect_timer = sleep_until(connect_deadline);
    tokio::pin!(connect_timer);
    // Before handoff, stopping wins even over a ready connection. No HTTP
    // request has been submitted, regardless of TCP/TLS progress.
    let established = tokio::select! {
        biased;
        () = &mut cancelled => return Err(SetupFailure::Stopped(StartRefusal::Cancelled)),
        () = &mut timer => return Err(SetupFailure::Stopped(StartRefusal::DeadlineTooClose)),
        () = &mut connect_timer => return Err(SetupFailure::Connection(ConnectFailure::TimedOut)),
        result = connect => result,
    };
    // A setup poll can block (for example the platform certificate verifier).
    // Recheck clocks before accepting its result, even if its timer was pending
    // when the poll began. No late successful setup may extend either budget.
    if let Some(reason) = gate.stopped() {
        return Err(SetupFailure::Stopped(reason));
    }
    if Instant::now() >= connect_deadline {
        return Err(SetupFailure::Connection(ConnectFailure::TimedOut));
    }
    match established {
        Ok(connection) => Ok(connection),
        Err(error) => Err(SetupFailure::Connection(
            if error.kind() == std::io::ErrorKind::TimedOut {
                ConnectFailure::TimedOut
            } else {
                ConnectFailure::Failed
            },
        )),
    }
}

/// Consume the sole sender and drive its sole connection until classification.
pub(crate) async fn send_on<C, F, I>(
    (mut sender, connection): (
        http1::SendRequest<Full<Bytes>>,
        http1::Connection<I, Full<Bytes>>,
    ),
    request: http::Request<Full<Bytes>>,
    gate: &Gate<F>,
    limit: ResponseLimit,
    mut timer: Pin<&mut Sleep>,
    mut cancelled: Pin<&mut C>,
) -> Exchange
where
    C: Future<Output = ()>,
    F: Fn() -> bool,
    I: hyper::rt::Read + hyper::rt::Write + Unpin,
{
    // The real commit point: no await between this check and send_request,
    // which synchronously hands the request to this owned connection.
    if let Some(reason) = gate.stopped() {
        return not_started(reason);
    }
    let pending = sender.send_request(request);
    tokio::pin!(connection);
    let mut driver = Some(connection);
    // A ready reply wins after handoff. Driving the connection may read it,
    // but every transport write/flush checks the gate, including inside TLS.
    let response = tokio::select! {
        biased;
        result = drive(pending, &mut driver) => match result {
            Ok(response) => response,
            Err(_) => return Exchange::Unknown(UnknownReason::Transport),
        },
        () = &mut cancelled => return Exchange::Unknown(UnknownReason::Cancelled),
        () = &mut timer => return Exchange::Unknown(UnknownReason::Deadline),
    };
    let status = response.status();
    let headers = sensitive(response.headers());
    let body = read_body(response.into_body(), limit, timer, cancelled, &mut driver).await;
    Exchange::Responded(Response {
        status,
        headers,
        body,
    })
}

fn not_started(reason: StartRefusal) -> Exchange {
    Exchange::NotStarted(NotStarted::new(reason))
}

fn not_dispatched(failure: ConnectFailure) -> Exchange {
    Exchange::NotDispatched(NotDispatched::new(failure))
}

/// Drive both halves in this task. Re-poll the receiver after the connection
/// so a response it just published is observed before interruption. A completed
/// connection is never polled again; buffered response/body data remains usable.
async fn drive<F: Future, D: Future>(future: F, driver: &mut Option<Pin<&mut D>>) -> F::Output {
    tokio::pin!(future);
    poll_fn(|cx| {
        if let Poll::Ready(value) = future.as_mut().poll(cx) {
            return Poll::Ready(value);
        }
        if let Some(connection) = driver.as_mut()
            && connection.as_mut().poll(cx).is_ready()
        {
            *driver = None;
        }
        future.as_mut().poll(cx)
    })
    .await
}

fn sensitive(headers: &HeaderMap) -> HeaderMap {
    let mut copy = headers.clone();
    for value in copy.values_mut() {
        value.set_sensitive(true);
    }
    copy
}

async fn read_body<C: Future<Output = ()>, D: Future>(
    mut body: Incoming,
    limit: ResponseLimit,
    mut timer: Pin<&mut Sleep>,
    mut cancelled: Pin<&mut C>,
    driver: &mut Option<Pin<&mut D>>,
) -> Body {
    use hyper::body::Body as _;
    let limit = limit.bytes();
    let declared = body.size_hint().exact();
    if declared.is_some_and(|length| length > limit as u64) {
        return Body::TooLarge;
    }
    let capacity = declared.map_or(0, |length| (length as usize).min(PREALLOCATE_MAX));
    let mut buffer = Vec::with_capacity(capacity);
    loop {
        tokio::select! {
            biased;
            frame = drive(body.frame(), driver) => match frame {
                Some(Ok(frame)) => {
                    let Ok(chunk) = frame.into_data() else { continue };
                    let needed = buffer.len().saturating_add(chunk.len());
                    if needed > limit {
                        return Body::TooLarge;
                    }
                    if needed > buffer.capacity() {
                        let target = needed.max(buffer.capacity().saturating_mul(2)).min(limit);
                        buffer.reserve_exact(target - buffer.len());
                    }
                    buffer.extend_from_slice(&chunk);
                }
                None => {
                    buffer.shrink_to_fit();
                    return Body::Complete(Bytes::from(buffer));
                }
                Some(Err(_)) => return Body::Failed(BodyFailure::Transport),
            },
            () = &mut cancelled => return Body::Failed(BodyFailure::Cancelled),
            () = &mut timer => return Body::Failed(BodyFailure::Deadline),
        }
    }
}

#[cfg(test)]
mod tests;
