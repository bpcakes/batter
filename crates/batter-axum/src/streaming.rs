//! A library-owned lifetime for a response whose body outlives its handler.
//!
//! Admission's boundary ends when a response is constructed: the request
//! context is cancelled, the completion event is emitted and Hyper polls the
//! body on its own connection task. [`AdmittedRequest::into_streaming_response`]
//! gives that body an owner of its own instead.

use crate::{AdmittedRequest, CorrelationId, observation::ObservationState};
use axum::{
    body::{Body, Bytes, HttpBody},
    response::{IntoResponse, Response},
};
use batter_core::{
    ConfigurationError,
    lifecycle::ShutdownSignal,
    operation::{Interruption, OperationContext, OperationOwner, RootDeadline},
};
use http_body::{Frame, SizeHint};
use std::{
    fmt,
    future::Future,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};
use tokio::time::{Instant, Sleep};

/// Validated bounds for one owned response body.
///
/// `total` bounds the whole body from the moment the handler hands it over.
/// `idle` bounds each wait for the body's next frame, so a source that stops
/// producing is ended long before a generous total. Both are required: a body
/// that outlives its handler has no other bound.
///
/// A stream meant to run indefinitely still needs a total: choose the longest
/// connection the service accepts. Reaching it truncates the body and is
/// logged at WARN, so a stream whose peer should instead see a clean end
/// finishes by itself shortly before, for example when
/// [`BodyScope::context`]'s `remaining()` runs low.
///
/// ```
/// use batter_axum::BodyBudget;
/// use std::time::Duration;
///
/// let budget = BodyBudget::new(Duration::from_secs(3600), Duration::from_secs(30))?;
/// assert_eq!(budget.total(), Duration::from_secs(3600));
/// assert_eq!(budget.idle(), Duration::from_secs(30));
/// // A wait that could never expire before the total is a configuration error.
/// assert!(BodyBudget::new(Duration::from_secs(1), Duration::from_secs(2)).is_err());
/// assert!(BodyBudget::new(Duration::ZERO, Duration::ZERO).is_err());
/// # Ok::<(), batter_core::ConfigurationError>(())
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BodyBudget {
    total: Duration,
    idle: Duration,
}

impl BodyBudget {
    /// Validate a positive total that Tokio can represent as a deadline and a
    /// positive idle bound no longer than it.
    pub fn new(total: Duration, idle: Duration) -> Result<Self, ConfigurationError> {
        const TOTAL: &str = "HTTP body total budget";
        const IDLE: &str = "HTTP body idle budget";
        if total.is_zero() {
            return Err(ConfigurationError::Zero(TOTAL));
        }
        if idle.is_zero() {
            return Err(ConfigurationError::Zero(IDLE));
        }
        if total > Duration::from_secs(365 * 24 * 60 * 60)
            || Instant::now().checked_add(total).is_none()
        {
            return Err(ConfigurationError::TooLarge(TOTAL));
        }
        if idle > total {
            return Err(ConfigurationError::TooLarge(IDLE));
        }
        Ok(Self { total, idle })
    }

    /// The validated bound on the whole body.
    pub const fn total(self) -> Duration {
        self.total
    }

    /// The validated bound on each wait for the next frame.
    pub const fn idle(self) -> Duration {
        self.idle
    }
}

/// What a response body receives in place of the request's context.
///
/// Only [`AdmittedRequest::into_streaming_response`] creates one. Move it into
/// the stream that produces the body. A literal naming every field is
/// otherwise well typed, so privacy is its only error:
///
/// ```compile_fail,E0451
/// use batter_axum::{BodyScope, CorrelationId};
/// use batter_core::{lifecycle::ShutdownSignal, operation::OperationContext};
///
/// fn forge(
///     context: OperationContext,
///     shutdown: ShutdownSignal,
///     correlation_id: CorrelationId,
/// ) -> BodyScope {
///     BodyScope { context, shutdown, correlation_id }
/// }
/// ```
pub struct BodyScope {
    context: OperationContext,
    shutdown: ShutdownSignal,
    correlation_id: CorrelationId,
}

impl BodyScope {
    /// The body's operation context. Its deadline is the body's total bound.
    /// It is cancelled by process forced cancellation and when the body ends
    /// or is destroyed, and not when the handler returns.
    ///
    /// Run each step of the stream through it, exactly as a handler runs work
    /// through [`AdmittedRequest::context`], and give a single step a tighter
    /// bound with [`OperationContext::child`]. A clone kept beyond the body
    /// observes the body's end as cancellation.
    ///
    /// The library checks this context before it polls the stream. When the
    /// body's own bound or forced cancellation is the cause, the peer
    /// therefore sees the library's truncation and a step's `Interrupted`
    /// result is normally never reached; it remains reachable for a step's own
    /// child deadline. Either way, end the stream with an error rather than
    /// normally, so the message is not mistaken for a complete one. The
    /// transport stops polling after the first error.
    pub fn context(&self) -> &OperationContext {
        &self.context
    }

    /// Read-only drain and forced-cancellation observation.
    ///
    /// Ending at drain is the stream's decision: an event stream usually ends
    /// when [`ShutdownSignal::draining`] resolves so its client reconnects
    /// elsewhere, while a download continues through drain. Either way the
    /// library ends the body at forced cancellation.
    pub fn shutdown(&self) -> &ShutdownSignal {
        &self.shutdown
    }

    /// The server-generated correlation of the request this body answers.
    pub fn correlation_id(&self) -> &CorrelationId {
        &self.correlation_id
    }
}

impl fmt::Debug for BodyScope {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BodyScope")
            .field("context", &self.context)
            .field("correlation_id", &self.correlation_id)
            .finish_non_exhaustive()
    }
}

impl AdmittedRequest {
    /// Build a response whose body the library owns after the handler returns.
    ///
    /// Use this for every body that is still being produced when the handler
    /// returns: server-sent events, a chunked export, a proxied download. The
    /// request's own context is cancelled at that moment, so a stream that
    /// kept [`Self::context`] would find its first step already interrupted.
    /// `build` instead receives a [`BodyScope`] and returns anything that is
    /// [`IntoResponse`]; the body of that response is then owned:
    ///
    /// - Its context lives until the body ends, `budget`'s total bound passes
    ///   or the process forces cancellation. It starts from this request's
    ///   admission, so a body that begins during drain is not refused.
    /// - A wait for the next frame longer than `budget`'s idle bound, the total
    ///   bound and forced cancellation each end the body with an error. The
    ///   peer sees a truncated message, never a final chunk it could mistake
    ///   for a complete one.
    /// - The body is polled and destroyed under the tracing dispatcher current
    ///   here and inside the request's span, on whichever task the transport
    ///   uses.
    /// - Exactly one `HTTP response body finished` event carries the request
    ///   id, the bytes yielded, the duration and a `body_outcome` of
    ///   `completed`, `failed`, `deadline_exceeded`, `idle_timeout`,
    ///   `cancelled`, `panicked` or `dropped`. A returned stream error, a
    ///   bound, forced cancellation and a panic emit WARN; completion and
    ///   `dropped` emit INFO. `dropped` names a body its holder discarded
    ///   before its end without the library, a panic or the stream ending it:
    ///   a peer that went away, a HEAD response whose body Axum strips, or a
    ///   response discarded because response construction ended without it.
    ///   The request's own completion event, with its method and status,
    ///   tells these apart. `panicked` names a builder that never
    ///   returned, a poll of the body that unwound, or destruction while the
    ///   thread unwinds, such as a handler panicking after this call; it
    ///   carries no panic payload. The request's own completion event is
    ///   unchanged and still describes response construction only.
    ///
    /// This consumes the admitted request: the handler's authority ends where
    /// the body's begins. The authority to start a body belongs to the
    /// request, not to one extracted value: it is single-use across every
    /// extraction and valid only while the request's response is being
    /// constructed. A second call, or a call on a request kept after its
    /// response was returned, still builds the response, but its scope is
    /// already cancelled: a body with content ends at its first poll,
    /// truncated, and every refused start is reported `cancelled`, including
    /// an empty body the transport never polls. Limits: the bounds are observed only when the
    /// transport polls the body, so a peer that stops reading is ended by the
    /// registered server's forced connection close at shutdown, not by them;
    /// an upgraded connection is not a body; and admission permits such as a
    /// bulkhead's are not held for the body's lifetime.
    ///
    /// ```
    /// use axum::{body::{Body, Bytes}, http::header, response::Response, routing::get};
    /// use batter_axum::{AdmittedRequest, BodyBudget, GuardedRouter};
    /// use batter_core::operation::OperationError;
    /// use futures_util::stream;
    /// use std::{convert::Infallible, time::Duration};
    ///
    /// async fn export(admitted: AdmittedRequest) -> Response {
    ///     let budget = BodyBudget::new(Duration::from_secs(600), Duration::from_secs(30))
    ///         .expect("a positive total no shorter than the idle bound");
    ///     admitted.into_streaming_response(budget, |scope| {
    ///         let chunks = stream::unfold((scope, 0_u32), |(scope, index)| async move {
    ///             if index == 3 {
    ///                 return None;
    ///             }
    ///             // Each step is bounded and cancelled by the body's context.
    ///             let chunk = scope
    ///                 .context()
    ///                 .run("export.chunk", |_step| async move {
    ///                     Ok::<_, Infallible>(Bytes::from(format!("chunk {index}\n")))
    ///                 })
    ///                 .await;
    ///             match chunk {
    ///                 Ok(chunk) => Some((Ok(chunk), (scope, index + 1))),
    ///                 // Ending with an error truncates the message visibly.
    ///                 Err(OperationError::Interrupted(reason)) => {
    ///                     Some((Err(std::io::Error::other(reason.to_string())), (scope, 3)))
    ///                 }
    ///                 Err(OperationError::Failed(never)) => match never {},
    ///             }
    ///         });
    ///         ([(header::CONTENT_TYPE, "text/plain")], Body::from_stream(chunks))
    ///     })
    /// }
    ///
    /// let guarded: GuardedRouter = GuardedRouter::new().route("/export", get(export));
    /// # let _ = guarded;
    /// ```
    ///
    /// The admitted request cannot be used again afterwards:
    ///
    /// ```compile_fail,E0382
    /// use batter_axum::{AdmittedRequest, BodyBudget};
    ///
    /// fn twice(admitted: AdmittedRequest, budget: BodyBudget) {
    ///     let _first = admitted.into_streaming_response(budget, |_scope| ());
    ///     let _context = admitted.context();
    /// }
    /// ```
    ///
    /// Raw durations are not a budget:
    ///
    /// ```compile_fail,E0308
    /// use batter_axum::AdmittedRequest;
    /// use std::time::Duration;
    ///
    /// fn unbounded(admitted: AdmittedRequest) {
    ///     let _ = admitted.into_streaming_response(Duration::MAX, |_scope| ());
    /// }
    /// ```
    pub fn into_streaming_response<R, F>(self, budget: BodyBudget, build: F) -> Response
    where
        F: FnOnce(BodyScope) -> R,
        R: IntoResponse,
    {
        let admission = self.into_admission();
        let started = Instant::now();
        // The budget was representable when validated; an unrepresentable
        // deadline now yields an already expired body rather than a panic.
        let deadline = started.checked_add(budget.total).unwrap_or(started);
        let (owner, refused) = if let Some(continuation) = admission.start_continuation() {
            (continuation.start(RootDeadline::at(deadline)), false)
        } else {
            // A second call for this request, or one after its response was
            // constructed: admission's decision no longer covers new work, so
            // a body with content ends at its first poll, truncated.
            let refused = OperationOwner::at(RootDeadline::at(started));
            refused.cancel();
            (refused, true)
        };
        let scope = BodyScope {
            context: owner.context().clone(),
            shutdown: admission.shutdown.clone(),
            correlation_id: admission.correlation_id().clone(),
        };
        // Owned before the application builds anything, so a panicking builder
        // still cancels the scope it was given and is still observed.
        let mut owned = OwnedBody {
            inner: None,
            owner,
            dispatch: tracing::dispatcher::get_default(Clone::clone),
            context: admission
                .observation
                .as_ref()
                .and_then(ObservationState::context),
            correlation_id: admission.correlation_id().clone(),
            started,
            idle: budget.idle,
            wake: None,
            bytes: 0,
            ended: false,
            polling: false,
            refused,
        };
        let (parts, body) = build(scope).into_response().into_parts();
        owned.inner = Some(body);
        Response::from_parts(parts, Body::new(owned))
    }
}

/// How an owned body ended, as its completion event names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BodyEnd {
    Completed,
    Failed,
    DeadlineExceeded,
    IdleTimeout,
    Cancelled,
    Panicked,
    Dropped,
}

impl BodyEnd {
    const fn label(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::DeadlineExceeded => "deadline_exceeded",
            Self::IdleTimeout => "idle_timeout",
            Self::Cancelled => "cancelled",
            Self::Panicked => "panicked",
            Self::Dropped => "dropped",
        }
    }

    const fn interrupted(reason: Interruption) -> Self {
        match reason {
            Interruption::DeadlineExceeded => Self::DeadlineExceeded,
            Interruption::Cancelled => Self::Cancelled,
        }
    }
}

/// The error a library-ended body yields, so the transport truncates the
/// message. It names the bound and nothing from the application.
#[derive(Debug)]
struct BodyInterrupted(BodyEnd);

impl fmt::Display for BodyInterrupted {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "response body ended: {}", self.0.label())
    }
}

impl std::error::Error for BodyInterrupted {}

/// The wakeups a pending body needs, allocated on its first pending poll.
struct Wake {
    timer: Pin<Box<Sleep>>,
    cancelled: Pin<Box<dyn Future<Output = ()> + Send>>,
    /// Whether the timer currently describes this wait.
    armed: bool,
    /// Whether the armed instant is the idle bound rather than the total.
    idle_first: bool,
}

struct OwnedBody {
    /// `None` while the application is still building the response and after
    /// the body ended.
    inner: Option<Body>,
    owner: OperationOwner,
    dispatch: tracing::Dispatch,
    context: Option<tracing::Span>,
    correlation_id: CorrelationId,
    started: Instant,
    idle: Duration,
    wake: Option<Wake>,
    bytes: u64,
    ended: bool,
    /// Set while the application's body is being polled, so a poll that
    /// unwinds is recognised when the transport destroys the body afterwards.
    polling: bool,
    /// The request's body-start authority was already spent or expired. An
    /// empty body is never polled, so the refusal is reported on destruction.
    refused: bool,
}

impl OwnedBody {
    /// Record the end once: cancel the scope, emit the event, then destroy the
    /// application's body, all under the captured dispatcher and span.
    fn finish(&mut self, end: BodyEnd) {
        if self.ended {
            return;
        }
        self.ended = true;
        self.wake = None;
        let duration_ms = self.started.elapsed().as_secs_f64() * 1_000.0;
        // Closing a span releases its parent through the current default
        // dispatcher, so the span itself must also be released under ours.
        let context = self.context.take();
        let parent = context.as_ref().and_then(tracing::Span::id);
        tracing::dispatcher::with_default(&self.dispatch, || {
            {
                let _entered = context.as_ref().map(tracing::Span::enter);
                // Escaped clones and the stream's own destructors see the end.
                self.owner.cancel();
                let request_id = self.correlation_id.as_str();
                // The event precedes application destructors, which may panic.
                if matches!(end, BodyEnd::Completed | BodyEnd::Dropped) {
                    tracing::event!(
                        target: "batter",
                        parent: parent,
                        tracing::Level::INFO,
                        request_id,
                        body_outcome = end.label(),
                        bytes = self.bytes,
                        duration_ms,
                        "HTTP response body finished"
                    );
                } else {
                    tracing::event!(
                        target: "batter",
                        parent: parent,
                        tracing::Level::WARN,
                        request_id,
                        body_outcome = end.label(),
                        bytes = self.bytes,
                        duration_ms,
                        "HTTP response body finished"
                    );
                }
                drop(self.inner.take());
            }
            drop(context);
        });
    }

    fn interrupt(&mut self, end: BodyEnd) -> Poll<Option<Result<Frame<Bytes>, axum::Error>>> {
        self.finish(end);
        Poll::Ready(Some(Err(axum::Error::new(BodyInterrupted(end)))))
    }

    /// Register this task for forced cancellation and the nearer of the idle
    /// and total bounds, ending the body if either is already due.
    fn wait(&mut self, cx: &mut Context<'_>) -> Option<BodyEnd> {
        let context = self.owner.context();
        let wake = self.wake.get_or_insert_with(|| {
            let cancellation = context.clone();
            Wake {
                timer: Box::pin(tokio::time::sleep_until(context.deadline())),
                cancelled: Box::pin(async move { cancellation.cancelled().await }),
                armed: false,
                idle_first: false,
            }
        });
        if wake.cancelled.as_mut().poll(cx).is_ready() {
            return Some(BodyEnd::Cancelled);
        }
        if !wake.armed {
            let total = context.deadline();
            let idle = Instant::now().checked_add(self.idle).unwrap_or(total);
            wake.idle_first = idle < total;
            wake.timer.as_mut().reset(idle.min(total));
            wake.armed = true;
        }
        if wake.timer.as_mut().poll(cx).is_ready() {
            return Some(if wake.idle_first {
                BodyEnd::IdleTimeout
            } else {
                BodyEnd::DeadlineExceeded
            });
        }
        None
    }
}

impl HttpBody for OwnedBody {
    type Data = Bytes;
    type Error = axum::Error;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, axum::Error>>> {
        let this = self.get_mut();
        if this.ended {
            return Poll::Ready(None);
        }
        // A source that is always ready must not outrun its bounds.
        if let Err(reason) = this.owner.context().check() {
            return this.interrupt(BodyEnd::interrupted(reason));
        }
        let Some(inner) = this.inner.as_mut() else {
            return Poll::Ready(None);
        };
        this.polling = true;
        let polled = tracing::dispatcher::with_default(&this.dispatch, || {
            let _entered = this.context.as_ref().map(tracing::Span::enter);
            Pin::new(inner).poll_frame(cx)
        });
        this.polling = false;
        match polled {
            Poll::Ready(Some(Ok(frame))) => {
                if let Some(data) = frame.data_ref() {
                    this.bytes = this.bytes.saturating_add(data.len() as u64);
                }
                if let Some(wake) = &mut this.wake {
                    wake.armed = false;
                }
                Poll::Ready(Some(Ok(frame)))
            }
            Poll::Ready(Some(Err(error))) => {
                this.finish(BodyEnd::Failed);
                Poll::Ready(Some(Err(error)))
            }
            Poll::Ready(None) => {
                this.finish(BodyEnd::Completed);
                Poll::Ready(None)
            }
            Poll::Pending => match this.wait(cx) {
                Some(end) => this.interrupt(end),
                None => Poll::Pending,
            },
        }
    }

    fn is_end_stream(&self) -> bool {
        self.ended || self.inner.as_ref().is_some_and(HttpBody::is_end_stream)
    }

    fn size_hint(&self) -> SizeHint {
        self.inner
            .as_ref()
            .map_or_else(|| SizeHint::with_exact(0), HttpBody::size_hint)
    }
}

impl Drop for OwnedBody {
    fn drop(&mut self) {
        if self.ended {
            return;
        }
        // Only an unwinding builder leaves no body without ending it. A
        // transport destroys a body whose poll panicked only after catching
        // the unwind, so the flag, not `panicking`, names that case. A
        // transport that learned the end from `is_end_stream` never polls for
        // it, so a finished source is still a completed body.
        let end = if self.inner.is_none() || self.polling || std::thread::panicking() {
            BodyEnd::Panicked
        } else if self.refused {
            BodyEnd::Cancelled
        } else if self.inner.as_ref().is_some_and(HttpBody::is_end_stream) {
            BodyEnd::Completed
        } else {
            match self.owner.context().check() {
                Err(reason) => BodyEnd::interrupted(reason),
                Ok(()) => BodyEnd::Dropped,
            }
        };
        self.finish(end);
    }
}
