//! A server-sent event stream that the library owns after its handler returns.

use axum::response::{
    Response,
    sse::{Event, Sse},
};
use batter::axum::{AdmittedRequest, BodyBudget, BodyScope};
use batter::operation::OperationError;
use futures_util::stream;
use std::{convert::Infallible, io, time::Duration};

/// The handler only hands the body over. The request's own context ends when
/// this function returns, so the stream receives a [`BodyScope`] instead.
pub async fn events(admitted: AdmittedRequest) -> Response {
    // Both bounds are deliberate: an hour for the whole stream, and thirty
    // seconds for any single wait on the source.
    let budget = BodyBudget::new(Duration::from_secs(3600), Duration::from_secs(30))
        .expect("constant body budget is valid");
    admitted.into_streaming_response(budget, |scope| {
        Sse::new(stream::unfold(Some((scope, 0_u64)), next_event))
    })
}

type Streaming = Option<(BodyScope, u64)>;

async fn next_event(state: Streaming) -> Option<(io::Result<Event>, Streaming)> {
    let (scope, sequence) = state?;
    let read = tokio::select! {
        biased;
        // Ending at drain is this stream's choice, so its client reconnects to
        // another instance. A download would simply not watch for drain.
        () = scope.shutdown().draining() => {
            let closing = Event::default().event("closing").data("draining");
            return Some((Ok(closing), None));
        }
        // Each step runs through the body's context, which bounds it and
        // carries process forced cancellation.
        read = scope.context().run("demo.event", |_step| async move {
            // Demonstration only: replace this wait with a real source.
            tokio::time::sleep(Duration::from_millis(100)).await;
            Ok::<_, Infallible>(sequence)
        }) => read,
    };
    match read {
        Ok(value) => {
            let event = Event::default().data(value.to_string());
            Some((Ok(event), Some((scope, sequence + 1))))
        }
        // An error, not a normal end, so an HTTP/1.1 peer sees a truncated
        // stream. An HTTP/1.0 peer cannot: this body declares no length.
        Err(OperationError::Interrupted(reason)) => {
            Some((Err(io::Error::other(reason.to_string())), None))
        }
        Err(OperationError::Failed(never)) => match never {},
    }
}
