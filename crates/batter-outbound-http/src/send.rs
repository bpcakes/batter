//! The single send path and its transport classification.

use crate::{
    exchange::{
        Body, BodyFailure, ConnectFailure, Exchange, NotDispatched, NotStarted, Response,
        StartRefusal, UnknownReason,
    },
    limits::ResponseLimit,
};
use bytes::Bytes;
use http::HeaderMap;
use std::{future::Future, pin::Pin, time::Duration};
use tokio::time::{Instant, Sleep, sleep_until};

/// Time kept free at the end of the caller's deadline. The exchange returns
/// its classification at least this long before that deadline, apart from
/// scheduling delay and any poll that blocks, such as macOS certificate
/// verification (see the crate's Limits; source-inspected), so an outer timer
/// at the same deadline does not discard it. It is not a recording budget:
/// reserve finalization time separately.
pub const RESERVE: Duration = Duration::from_millis(50);

/// The shortest window the exchange will send into. With less time left, a
/// request could be written but almost never answered, turning a refusal that
/// proves nothing was sent into an [`Exchange::Unknown`].
pub const MINIMUM_ATTEMPT: Duration = Duration::from_millis(50);

/// Preallocation cap, so a large declared `Content-Length` cannot reserve
/// memory before any byte arrives.
const PREALLOCATE_MAX: usize = 1024 * 1024;

pub(crate) async fn run<C>(
    http: reqwest::Client,
    request: reqwest::Request,
    deadline: Instant,
    limit: ResponseLimit,
    cancelled: C,
) -> Exchange
where
    C: Future<Output = ()>,
{
    let Some(exchange_deadline) = deadline.checked_sub(RESERVE) else {
        return Exchange::NotStarted(NotStarted::new(StartRefusal::DeadlineTooClose));
    };
    // Checked immediately before the first poll below, with no await between.
    if exchange_deadline.saturating_duration_since(Instant::now()) < MINIMUM_ATTEMPT {
        return Exchange::NotStarted(NotStarted::new(StartRefusal::DeadlineTooClose));
    }
    let timer = sleep_until(exchange_deadline);
    let pending = http.execute(request);
    tokio::pin!(timer, pending, cancelled);
    // A ready result wins over the timer and cancellation: polling a request
    // that was already handed to the client cannot start another request.
    let response = tokio::select! {
        biased;
        result = &mut pending => match result {
            Ok(response) => response,
            Err(error) => return classify(&error),
        },
        () = &mut cancelled => return Exchange::Unknown(UnknownReason::Cancelled),
        () = &mut timer => return Exchange::Unknown(UnknownReason::Deadline),
    };
    let status = response.status();
    let headers = sensitive(response.headers());
    let body = read_body(response, limit, timer, cancelled).await;
    Exchange::Responded(Response {
        status,
        headers,
        body,
    })
}

/// Only hyper-util's connect kind proves that this attempt wrote nothing. It is
/// assigned to connector failures (DNS, TCP, TLS, connect timeout) and pool
/// checkout failures before the request is handed to a connection. Redirects
/// are disabled, so no earlier hop can have been delivered.
fn classify(error: &reqwest::Error) -> Exchange {
    if error.is_connect() {
        let failure = if error.is_timeout() {
            ConnectFailure::TimedOut
        } else {
            ConnectFailure::Failed
        };
        Exchange::NotDispatched(NotDispatched::new(failure))
    } else {
        Exchange::Unknown(UnknownReason::Transport)
    }
}

fn sensitive(headers: &HeaderMap) -> HeaderMap {
    let mut copy = headers.clone();
    for value in copy.values_mut() {
        value.set_sensitive(true);
    }
    copy
}

async fn read_body<C>(
    mut response: reqwest::Response,
    limit: ResponseLimit,
    mut timer: Pin<&mut Sleep>,
    mut cancelled: Pin<&mut C>,
) -> Body
where
    C: Future<Output = ()>,
{
    let limit = limit.bytes();
    let declared = response.content_length();
    if declared.is_some_and(|length| length > limit as u64) {
        return Body::TooLarge;
    }
    let capacity = declared.map_or(0, |length| (length as usize).min(PREALLOCATE_MAX));
    let mut buffer = Vec::with_capacity(capacity);
    loop {
        tokio::select! {
            biased;
            chunk = response.chunk() => match chunk {
                Ok(Some(chunk)) => {
                    let needed = buffer.len().saturating_add(chunk.len());
                    if needed > limit {
                        return Body::TooLarge;
                    }
                    if needed > buffer.capacity() {
                        // Grow geometrically but never past the limit, so the
                        // buffer itself stays within it.
                        let target = needed.max(buffer.capacity().saturating_mul(2)).min(limit);
                        buffer.reserve_exact(target - buffer.len());
                    }
                    buffer.extend_from_slice(&chunk);
                }
                Ok(None) => {
                    // Release spare capacity rather than keep it alive in `Bytes`.
                    buffer.shrink_to_fit();
                    return Body::Complete(Bytes::from(buffer));
                }
                Err(_) => return Body::Failed(BodyFailure::Transport),
            },
            () = &mut cancelled => return Body::Failed(BodyFailure::Cancelled),
            () = &mut timer => return Body::Failed(BodyFailure::Deadline),
        }
    }
}
