//! One poll of an owned body's source: the bounds around it, what each
//! answer means for the body, and the wait a pending answer leaves.

use super::{BodyEnd, OwnedBody};
use axum::body::{Bytes, HttpBody};
use http_body::Frame;
use std::{
    pin::Pin,
    task::{Context, Poll},
};
use tokio::time::Instant;

impl OwnedBody {
    /// One poll of a body that has not ended.
    pub(super) fn poll_source(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, axum::Error>>> {
        let this = self;
        // A holder that caught an unwinding call and polls on gets the end
        // that call left, as an error, and the source is not entered again.
        if this.unwound() {
            // This poll delivers the error, so none is owed afterwards and
            // the body is then at its end like any other.
            this.polling = false;
            this.calling.set(false);
            return this.interrupt(BodyEnd::Panicked);
        }
        // A source that is always ready must not outrun its bounds, and one
        // that became ready after its armed wait expired is not accepted.
        if let Some(end) = this.overdue(None) {
            return this.interrupt(end);
        }
        // A wait for the next frame starts with the poll that first asks for
        // it, so synchronous work the source does in that poll counts too.
        let asked = this.wake.is_none().then(Instant::now);
        let Some(inner) = this.inner.as_mut() else {
            return Poll::Ready(None);
        };
        // The source's poll is application code; see `application`.
        let poll = || {
            let _entered = this.context.as_ref().map(tracing::Span::enter);
            Pin::new(inner).poll_frame(cx)
        };
        let polled = match &this.dispatch {
            Some(dispatch) => tracing::dispatcher::with_default(dispatch, poll),
            None => poll(),
        };
        // Nor is a frame, end or error the source returned from a poll that
        // outlasted a bound: the check above ran before that poll started, and
        // the bound, not the source, then ended the body.
        let overdue = if polled.is_ready() {
            this.overdue(asked)
        } else {
            None
        };
        if let Some(end) = overdue {
            return this.reject(end, polled);
        }
        match polled {
            // Hyper discards an empty data frame and polls again at once,
            // without yielding, so one is no progress: it neither restarts the
            // idle wait nor reaches the transport, and the task yields. A final
            // empty frame ends the message at once, as Hyper ends it; the end
            // is asked only after the frame is destroyed under protection.
            Poll::Ready(Some(Ok(frame))) if frame.data_ref().is_some_and(Bytes::is_empty) => {
                this.discard(frame);
                // The frame's destructor and the end query are application
                // code as well, and either can outlast a bound the checks
                // around the poll found unexpired. An expired body asks its
                // source nothing more, so each is checked before the next.
                if let Some(end) = this.overdue(asked) {
                    return this.interrupt(end);
                }
                let ended = this.inspect(HttpBody::is_end_stream).unwrap_or(false);
                if let Some(end) = this.overdue(asked) {
                    return this.interrupt(end);
                }
                if ended {
                    let end = this.yielded_end();
                    this.finish(end);
                    return Poll::Ready(None);
                }
                if let Some(end) = this.wait(cx, asked) {
                    return this.interrupt(end);
                }
                cx.waker().wake_by_ref();
                Poll::Pending
            }
            Poll::Ready(Some(Ok(frame))) => {
                if let Some(data) = frame.data_ref() {
                    this.bytes = this.bytes.saturating_add(data.len() as u64);
                }
                if frame.is_trailers() {
                    // Trailers are a body's last frame. Hyper destroys the
                    // body once it has them, but another holder may poll on,
                    // so the body ends here and never asks the source again.
                    let end = this.yielded_end();
                    return Poll::Ready(this.finish_holding(end, frame).map(Ok));
                }
                if let Some(wait) = this.wake.take() {
                    this.application(|| drop(wait));
                }
                Poll::Ready(Some(Ok(frame)))
            }
            Poll::Ready(Some(Err(error))) => {
                Poll::Ready(this.finish_holding(BodyEnd::Failed, error).map(Err))
            }
            Poll::Ready(None) => {
                this.finish(this.yielded_end());
                Poll::Ready(None)
            }
            Poll::Pending => match this.wait(cx, asked) {
                Some(end) => this.interrupt(end),
                None => Poll::Pending,
            },
        }
    }
}
