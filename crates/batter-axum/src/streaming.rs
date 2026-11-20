//! A library-owned lifetime for a response whose body outlives its handler.
//!
//! Admission's boundary ends when a response is constructed: the request
//! context is cancelled, the completion event is emitted and Hyper polls the
//! body on its own connection task. [`AdmittedRequest::into_streaming_response`]
//! gives that body an owner of its own instead.

mod budget;
mod end;
mod poll;
mod scope;
mod start;

pub use budget::BodyBudget;
pub use scope::BodyScope;
pub(crate) use start::BodyStart;

use crate::{AdmittedRequest, CorrelationId, observation::ObservationState};
use axum::{
    body::{Body, Bytes, HttpBody},
    response::{IntoResponse, Response},
};
use batter_core::operation::{OperationOwner, RootDeadline};
use end::{BodyEnd, BodyInterrupted, Ending, Wake};
use http_body::{Frame, SizeHint};
use std::{
    cell::Cell,
    future::Future,
    panic::{AssertUnwindSafe, catch_unwind, resume_unwind},
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};
use tokio::time::Instant;

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
    ///   bound and forced cancellation each end the body with an error. Both
    ///   bounds are checked before and after each poll of the source, so a frame
    ///   ready or returned only after its wait expired is never accepted, and
    ///   an error returned then is reported as that bound; such a rejected
    ///   answer is destroyed like the body, below. A wait starts with the poll
    ///   that first asks for the next frame, so synchronous work the source
    ///   does inside that poll counts; a poll that never returns cannot be
    ///   ended. An empty data frame is no progress: it does not restart the
    ///   idle wait, and a final one ends the message only if destroying it
    ///   and asking the source for its end stayed inside both bounds. Each
    ///   of these ends the body with an error at a poll: the library wakes
    ///   a poll it left pending, a holder not waiting on one meets the bound
    ///   at its next poll, and a body that is not polled again ends only
    ///   when it is destroyed, by the outcome rule below, which reads
    ///   cancellation and the total bound, not the idle bound. What the peer
    ///   sees after such an error is the transport's to decide. In the
    ///   HTTP/1.1 cases exercised, a response sent chunked and one framed
    ///   by a `Content-Length` its body had not yet produced, Hyper ended
    ///   the connection without completing the message. Over HTTP/1.0,
    ///   which Hyper answers an HTTP/1.0 request in, a response sent
    ///   without a `Content-Length` ends when the connection closes, so
    ///   that peer cannot tell the truncation from the message's end: make
    ///   sure the response carries a `Content-Length` where an HTTP/1.0
    ///   peer must detect it.
    /// - The body is polled, asked for its end and size, and destroyed under
    ///   the tracing dispatcher current here and inside the request's span, on
    ///   whichever task the transport uses. The completion event and the
    ///   request span itself stay with the request's subscriber even if the
    ///   handler switched subscribers before this call: the subscriber that
    ///   owns the span the request's observer published, which is its
    ///   `batter.http` span or, where that was filtered out, the span
    ///   current when the observer began. Where there is no such span, or
    ///   the observer has already finished, it is the subscriber the request
    ///   was admitted under while that still exists, and the one current at
    ///   this call otherwise. Once the request's observer has finished,
    ///   the library's own request records hold no subscriber strongly, and
    ///   a body that has ended, which is when its event is emitted, holds
    ///   none. A layer that copies another live
    ///   request's records into this one's extensions gives the body that
    ///   request's span and subscriber, which is not supported.
    /// - Exactly one `HTTP response body finished` event carries the request
    ///   id, the bytes yielded, the duration and a `body_outcome`.
    ///   `completed` and `dropped` emit INFO, every other outcome WARN. The
    ///   outcome follows from three
    ///   things only: the length the response `build` returns declares, read
    ///   once at this call; the bytes the source has yielded, to whoever
    ///   polled the body; and how the body ended.
    ///   - `deadline_exceeded`, `idle_timeout`, `cancelled`: the library
    ///     ended it, at a bound or at forced cancellation, or refused its
    ///     start; or its holder destroyed it, before its source's end and
    ///     before it had yielded a declared length, if it declared one,
    ///     after its scope was cancelled or its total bound had passed.
    ///   - `panicked`: the builder never returned; a poll of the body, the
    ///     destruction of an empty frame it yielded, or a query of its end or
    ///     size unwound, whether the hand-over, the transport or its
    ///     destruction asked; or it was destroyed while the thread unwinds,
    ///     such as a handler panicking after this call. No panic payload is
    ///     carried. A call that unwinds cancels the scope at once. A holder
    ///     that catches the unwind and polls on gets an error, and the
    ///     source is not entered again; the same holds, once, after an
    ///     ending that unwound with another outcome already reported.
    ///   - `failed`: the source returned an error, or reached its end short
    ///     of the declared length, so the application's own response
    ///     promised more than its source produced.
    ///   - `completed`: the source reached its end otherwise, or the body was
    ///     destroyed having yielded its declared length, because Hyper's
    ///     HTTP/1 connection, in the cases exercised, stops polling there
    ///     without asking for the end. A source reaches
    ///     its end by yielding it; by trailers, which end the body as it
    ///     yields them, so a holder that polls on finds the end without the
    ///     source being asked again; or by reporting it, when asked after an
    ///     empty data frame, which ends the body at once, or when the body
    ///     is destroyed.
    ///   - `dropped`: its holder destroyed it before any of those, its
    ///     scope still live.
    ///
    ///   The declared length is that response's `Content-Length`, counted
    ///   only as plain digits, repeated only with one value, else its body's
    ///   exact size; a response that carries no content (answering HEAD, a
    ///   2xx answer to CONNECT, or a 1xx, 204 or 304 status) declares none.
    /// - The outcome describes the body, not the message: nothing in it says
    ///   what a transport wrote or a peer received, and it does not detect a
    ///   response head a transport refused to write, whose body is judged by
    ///   the same rule as any other. The rule is the same in-process and on
    ///   any server; the outcome is not, because whether and when a holder
    ///   stops polling and destroys the body is the holder's choice. In the
    ///   HTTP/1 cases exercised, Hyper destroyed a body once it had taken
    ///   from it the length it framed the message by; a holder that collects the
    ///   body, such as a test reading the in-process response, polls on
    ///   into the source's end or a bound. So
    ///   an in-process outcome is not evidence of a served one. `dropped` is
    ///   what several different events leave behind: a peer that went away,
    ///   a body not yet at its end that Axum strips from a HEAD response, a
    ///   body still pending and
    ///   short of any declared length under a head the transport refused, a
    ///   transport that stopped at a length this call never saw, or a
    ///   response discarded because response construction ended without it.
    ///   The request's own completion event, with its method and status,
    ///   tells some of these apart, such as a HEAD request or a refused
    ///   status, but not a transport that stopped at such a length from a
    ///   peer that went away after the same bytes: both events read alike
    ///   for the two. That event is unchanged and still describes response
    ///   construction only.
    /// - Declare a length inside `build`, on the response it returns, for
    ///   the outcome to follow it. A `Content-Length` that a handler's tuple
    ///   or a layer adds after this call, and a size the body reports only
    ///   later, can still change how the transport frames the message but
    ///   are not read here: the outcome then follows what `build` declared, by the
    ///   rule above, and can part from the message either way. The crate
    ///   guide tabulates the cases exercised.
    ///
    /// This consumes the admitted request: the handler's authority ends where
    /// the body's begins. The authority to start a body belongs to the
    /// request, not to one extracted value: it is single-use across every
    /// extraction and valid only while the request's response is being
    /// constructed. A second call, or a call on a request kept after its
    /// response was returned, still builds the response, but its scope is
    /// already cancelled: a body with content ends at its first poll,
    /// truncated, and every refused start that does not panic is reported
    /// `cancelled`, including an empty body the transport never polls. After
    /// the request's observer finished that report carries no span and goes
    /// to the subscriber the request was admitted under while it still
    /// exists, else to the one current at this call. Limits: the bounds are observed only when the
    /// transport polls the body, so a peer that stops reading is ended by the
    /// registered server's forced connection close at shutdown, not by them;
    /// an upgraded connection is not a body; admission permits such as a
    /// bulkhead's are not held for the body's lifetime; and what is said
    /// above about unwinding covers the application's code for the body:
    /// the builder, the source and what it yields. A tracing subscriber or
    /// a task waker that panics is outside it, as neither `tracing` nor
    /// Tokio restores its own state after one.
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
    ///         .expect("a positive total longer than the idle bound");
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
    ///                 // An error truncates the message, visibly over HTTP/1.1.
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
        let deadline = started.checked_add(budget.total()).unwrap_or(started);
        let (owner, refused) =
            if let Some(owner) = admission.start_continuation(RootDeadline::at(deadline)) {
                (owner, false)
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
        // The handler may have switched subscribers since the request
        // began. The span its observer published names the dispatcher that
        // owns it, so the event and the span can never belong to different
        // subscribers. Where there is no such span, because that subscriber
        // enabled none or the observer finished before a start refused above,
        // the dispatcher this request was admitted under is the request's,
        // if it still exists; request metadata does not keep it alive.
        let observation = admission.observation.as_ref();
        let context = observation.and_then(ObservationState::span);
        let request_dispatch = context
            .as_ref()
            .and_then(|span| span.with_subscriber(|(_, owner)| owner.clone()))
            .or_else(|| admission.dispatch.upgrade());
        // Owned before the application builds anything, so a panicking builder
        // still cancels the scope it was given and is still observed.
        let mut owned = OwnedBody {
            inner: None,
            owner,
            dispatch: Some(tracing::dispatcher::get_default(Clone::clone)),
            context,
            request_dispatch,
            correlation_id: admission.correlation_id().clone(),
            started,
            idle: budget.idle(),
            wake: None,
            bytes: 0,
            declared: None,
            ended: false,
            polling: false,
            calling: Cell::new(false),
            refused,
        };
        let (parts, body) = build(scope).into_response().into_parts();
        // Owned before it is asked anything, so a query that unwinds still
        // ends the body under protection before the application's body is
        // destroyed.
        owned.inner = Some(body);
        // Read once, from the response the application built. Nothing later
        // changes it: the body cannot observe what a layer adds afterwards or
        // how a transport frames the message. The body is asked for its size
        // only where the response's status and headers leave it to decide.
        let size = || owned.inspect(HttpBody::size_hint)?.exact();
        let declared = admission.start.declared(&parts, size);
        owned.declared = declared;
        // The admission record goes before the response is assembled. It can
        // hold the last copy of request extensions the application supplied,
        // whose destructors are foreign code: if one unwinds here, the body
        // is still an owned local and ends under protection, where a
        // response already on its way out would be leaked with it.
        drop(admission);
        Response::from_parts(parts, Body::new(owned))
    }
}

struct OwnedBody {
    /// `None` while the application is still building the response and after
    /// the body ended.
    inner: Option<Body>,
    owner: OperationOwner,
    /// The dispatcher current at the hand-over, under which the application's
    /// body is polled, queried and destroyed. `None` once the body ended and
    /// nothing of the application's is left, so an exhausted body its holder
    /// keeps holds no dispatcher.
    dispatch: Option<tracing::Dispatch>,
    context: Option<tracing::Span>,
    /// The request's dispatcher, which the completion event and the span's
    /// release belong to: the one that owns `context` where there is such a
    /// span, since a span must close, and release its parent, in the
    /// registry that owns it, and otherwise the one the request was admitted
    /// under, if it still existed at the hand-over. `None` once the body
    /// ended, and where neither existed, when `dispatch` is used.
    request_dispatch: Option<tracing::Dispatch>,
    correlation_id: CorrelationId,
    started: Instant,
    idle: Duration,
    wake: Option<Wake>,
    bytes: u64,
    /// The length the built response declared at the hand-over: its
    /// `Content-Length`, else its body's exact size. A transport stops polling
    /// once it has written a declared length, without asking for the end.
    declared: Option<u64>,
    ended: bool,
    /// Set when a poll of this body unwound, so that is recognised when its
    /// holder polls on or destroys the body afterwards.
    polling: bool,
    /// Set while application code runs for the body outside a poll, a query
    /// of its end or size or the destruction of a frame it yielded, likewise.
    calling: Cell<bool>,
    /// The request's body-start authority was already spent or expired. An
    /// empty body is never polled, so the refusal is reported on destruction.
    refused: bool,
}

impl OwnedBody {
    /// Record the end once: cancel the scope, emit the event, then destroy the
    /// application's body, inside the request span. The event and the span's
    /// release run under the request's dispatcher, the application's body is
    /// destroyed under the one current at the hand-over; they differ only when
    /// the handler switched subscribers.
    fn finish(&mut self, end: BodyEnd) {
        self.finish_destroying(end, (), ());
    }

    /// End the body for `end` and hand back `value`, which the source
    /// produced and the poll has yet to return. Until then it stays the
    /// application's: if the ending unwinds, it is destroyed like the
    /// source, under the hand-over dispatcher and inside the request span.
    fn finish_holding<T>(&mut self, end: BodyEnd, value: T) -> Option<T> {
        self.finish_destroying(end, (), value)
    }

    /// End the body for `end`, destroying `rejected`, an answer the source
    /// gave too late, together with the source: after the event, under the
    /// captured dispatcher and inside the request span. `kept` is handed
    /// back if all of that returned, and destroyed the same way if not.
    fn finish_destroying<T, K>(&mut self, end: BodyEnd, rejected: T, kept: K) -> Option<K> {
        if self.ended {
            self.application(|| drop(rejected));
            return Some(kept);
        }
        self.ended = true;
        let duration_ms = self.started.elapsed().as_secs_f64() * 1_000.0;
        // Closing a span releases its parent through the current default
        // dispatcher, so the span must be released under the one that owns it.
        let context = self.context.take();
        let parent = context.as_ref().and_then(tracing::Span::id);
        // Taken, not borrowed: nothing of the application's is left to run
        // under it once the source is destroyed below.
        let handed_over = self.dispatch.take();
        let request = self
            .request_dispatch
            .take()
            .or_else(|| handed_over.clone())
            .unwrap_or_else(|| tracing::dispatcher::get_default(Clone::clone));
        let ending = tracing::dispatcher::with_default(&request, || {
            // The guard owns the source, a rejected answer and what the poll
            // has yet to return: each is destroyed under the hand-over
            // dispatcher and inside the span, also when the destruction of
            // another unwinds. The span is entered for the event, which
            // a span-scoped filter needs, stays entered through the source's
            // destruction, and is left and released here, under its owner,
            // which this closure's guard keeps current.
            let owned = (self.inner.take(), rejected, kept);
            let mut ending = Ending::armed(owned, self.wake.take(), handed_over);
            // Escaped clones and the stream's own destructors see the end.
            self.owner.cancel();
            ending.enter(context);
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
            ending.destroy_source();
            // The wait, with the waker of the task that polled this body,
            // goes once the source is destroyed, under the hand-over
            // dispatcher it was created under. The guard releases it too if
            // that destruction unwinds, so an ended body keeps no timer.
            ending.release_wait();
            ending.leave_span();
            ending
        });
        // An ended body holds no dispatcher: the request's goes here and the
        // guard releases the hand-over's.
        drop(request);
        ending.release()
    }

    /// How a source ended that its holder stopped polling without asking for
    /// the end, if it did. Hyper's HTTP/1 connection stops once it has the
    /// length it framed the message by and once `is_end_stream` reports the
    /// end; trailers, after which it stops too, already ended the body when
    /// it yielded them. Only the length
    /// declared at the hand-over is known here: a body dropped short of it,
    /// or with none declared, before its source's end tells nothing more.
    fn source_end(&self) -> Option<BodyEnd> {
        if self.inner.as_ref().is_some_and(HttpBody::is_end_stream) {
            return Some(self.yielded_end());
        }
        let produced = self.declared.is_some_and(|length| self.bytes >= length);
        produced.then_some(BodyEnd::Completed)
    }

    /// How a source that ended finished: short of the length its own
    /// response declared is a failure.
    fn yielded_end(&self) -> BodyEnd {
        if self.declared.is_some_and(|length| self.bytes < length) {
            BodyEnd::Failed
        } else {
            BodyEnd::Completed
        }
    }

    /// Run application code for this body under the captured dispatcher,
    /// which is held until the body has ended.
    fn application<T>(&self, application: impl FnOnce() -> T) -> T {
        match &self.dispatch {
            Some(dispatch) => tracing::dispatcher::with_default(dispatch, application),
            None => application(),
        }
    }

    /// Run application code under the captured dispatcher and span, as a
    /// poll is, marking it so one that unwinds is reported as a panic.
    fn call<T>(&self, application: impl FnOnce() -> T) -> T {
        self.calling.set(true);
        let answer = catch_unwind(AssertUnwindSafe(|| {
            self.application(|| {
                let _entered = self.context.as_ref().map(tracing::Span::enter);
                application()
            })
        }));
        match answer {
            Ok(answer) => {
                self.calling.set(false);
                answer
            }
            // That call was the body's last, whoever catches the unwind, so
            // contexts that escaped the stream must not stay live until the
            // holder polls or drops the body. The scope is cancelled here,
            // with the unwind caught, not from a destructor inside it.
            Err(panic) => {
                self.owner.cancel();
                resume_unwind(panic)
            }
        }
    }

    /// Whether a poll or other call of the application's body unwound. The
    /// body is then at its end, whoever caught the unwind.
    fn unwound(&self) -> bool {
        self.polling || self.calling.get()
    }

    /// Ask the application's body a question.
    fn inspect<T>(&self, query: impl FnOnce(&Body) -> T) -> Option<T> {
        self.inner.as_ref().map(|inner| self.call(|| query(inner)))
    }

    /// Destroy an answer the source gave that the transport never sees.
    fn discard<T>(&self, answer: T) {
        self.call(|| drop(answer));
    }

    fn interrupt(&mut self, end: BodyEnd) -> Poll<Option<Result<Frame<Bytes>, axum::Error>>> {
        self.reject(end, ())
    }

    /// End the body for `end` instead of passing on `rejected`, which the
    /// library, not the transport, then destroys under protection.
    fn reject<T>(
        &mut self,
        end: BodyEnd,
        rejected: T,
    ) -> Poll<Option<Result<Frame<Bytes>, axum::Error>>> {
        self.finish_destroying(end, rejected, ());
        Poll::Ready(Some(Err(axum::Error::new(BodyInterrupted(end)))))
    }

    /// The bound that has already ended the body, if any: cancellation, the
    /// total deadline, or the idle bound, measured by an armed wait or, when
    /// none is armed, from `asked`, the start of the poll that began the wait.
    fn overdue(&self, asked: Option<Instant>) -> Option<BodyEnd> {
        match self.owner.context().check() {
            Err(reason) => Some(BodyEnd::interrupted(reason)),
            Ok(()) => self.wake.as_ref().and_then(Wake::due).or_else(|| {
                let idle = asked?.checked_add(self.idle)?;
                (idle <= Instant::now()).then_some(BodyEnd::IdleTimeout)
            }),
        }
    }

    /// Register this task for forced cancellation and the nearer of the idle
    /// and total bounds, ending the body if either is already due. A wait not
    /// yet armed began at `asked`, or now.
    fn wait(&mut self, cx: &mut Context<'_>, asked: Option<Instant>) -> Option<BodyEnd> {
        let Self {
            dispatch,
            owner,
            wake,
            idle,
            ..
        } = self;
        let context = owner.context();
        let mut arm = || {
            let wake = wake.get_or_insert_with(|| {
                let cancellation = context.clone();
                let total = context.deadline();
                let since = asked.unwrap_or_else(Instant::now);
                let idle = since.checked_add(*idle).unwrap_or(total);
                Wake::new(idle, total, async move { cancellation.cancelled().await })
            });
            if wake.cancelled.as_mut().poll(cx).is_ready() {
                return Some(BodyEnd::Cancelled);
            }
            if wake.timer.as_mut().poll(cx).is_ready() {
                return Some(wake.timeout);
            }
            None
        };
        // A timer can carry tracing state of its own, as Tokio's does when
        // built with it, and that state belongs to the subscriber current
        // where the timer was created. Different tasks, under different
        // subscribers, may poll and destroy this body, so the wait is
        // created, armed, polled and destroyed under one dispatcher only:
        // the one the body was handed over under.
        match dispatch {
            Some(dispatch) => tracing::dispatcher::with_default(dispatch, arm),
            None => arm(),
        }
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
            // An ending that unwound never returned what its poll owed the
            // holder. A holder that caught the unwind gets an error once,
            // not a clean end; the source is gone and the event was emitted.
            this.calling.set(false);
            return Poll::Ready(
                std::mem::take(&mut this.polling)
                    .then(|| Err(axum::Error::new(BodyInterrupted(BodyEnd::Panicked)))),
            );
        }
        // A poll that unwinds was the body's last poll of its source: the
        // scope is cancelled at once and the unwind remembered, whoever
        // catches it.
        match catch_unwind(AssertUnwindSafe(|| this.poll_source(cx))) {
            Ok(polled) => polled,
            Err(panic) => {
                this.polling = true;
                this.owner.cancel();
                resume_unwind(panic)
            }
        }
    }

    fn is_end_stream(&self) -> bool {
        // After an unwinding call the source is asked nothing more, and the
        // body is not at a clean end: its next poll returns an error.
        !self.unwound() && (self.ended || self.inspect(HttpBody::is_end_stream).unwrap_or(false))
    }

    fn size_hint(&self) -> SizeHint {
        if self.unwound() {
            return SizeHint::default();
        }
        self.inspect(HttpBody::size_hint)
            .unwrap_or_else(|| SizeHint::with_exact(0))
    }
}

impl Drop for OwnedBody {
    fn drop(&mut self) {
        if self.ended {
            return;
        }
        // Only an unwinding builder leaves no body without ending it. A body
        // whose poll or other call unwound is destroyed only after a transport
        // caught the unwind, so the flags, not `panicking`, name that case.
        let end = if self.inner.is_none() || self.unwound() || std::thread::panicking() {
            BodyEnd::Panicked
        } else if self.refused {
            BodyEnd::Cancelled
        } else {
            // Asking the source for its end runs application code, so it runs
            // under the captured dispatcher and span, and one that unwinds
            // still ends the body under protection before the panic continues.
            let queried = self.application(|| {
                let _entered = self.context.as_ref().map(tracing::Span::enter);
                catch_unwind(AssertUnwindSafe(|| self.source_end()))
            });
            match queried {
                Ok(Some(end)) => end,
                Ok(None) => match self.owner.context().check() {
                    Err(reason) => BodyEnd::interrupted(reason),
                    Ok(()) => BodyEnd::Dropped,
                },
                // The payload is the application's too: the ending holds it,
                // and destroys it like the source if the ending unwinds.
                Err(panic) => match self.finish_holding(BodyEnd::Panicked, panic) {
                    Some(panic) => resume_unwind(panic),
                    None => return,
                },
            }
        };
        self.finish(end);
    }
}
