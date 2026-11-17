//! Completion of every accepted connection's application state.
//!
//! Pinned Axum 0.8.9 counts a connection finished when its task drops its
//! completion receiver, which it does before the pinned connection future
//! leaves scope (`serve/mod.rs:388-414`). After a connection error, such as the
//! forced-close gate's failed I/O, Hyper 1.11.1 still holds that connection's
//! in-flight service future and response body inside the future
//! (`proto/h1/dispatch.rs:123-141`) and destroys them only with it, response
//! body last. Axum's graceful return therefore proves neither destroyed, and a
//! slow destructor there could still be running when dependent cleanup starts.
//!
//! Each connection's service, each of its response futures and every response
//! body it produces hold a token instead, and the server completes only once
//! every token has been released. Each response future also computes, from the
//! response it hands Hyper, how Hyper frames that message, and its body makes
//! that framing current for every call Hyper makes into it, so an owned body
//! the message carries, through whatever layers, is judged by it.

use crate::streaming::Written;
use axum::{
    body::{Body, Bytes},
    extract::Request,
    http::{Method, Version},
    response::Response,
    serve::{IncomingStream, Listener},
};
use http_body::{Frame, SizeHint};
use pin_project_lite::pin_project;
use std::{
    convert::Infallible,
    future::Future,
    pin::Pin,
    task::{Context, Poll, ready},
};
use tokio_util::task::{TaskTracker, task_tracker::TaskTrackerToken};
use tower::Service;

/// The application state a server's accepted connections still hold, counted
/// by a [`TaskTracker`]: each holder keeps one of its tokens.
#[derive(Clone, Default)]
pub(super) struct Connections(TaskTracker);

impl Connections {
    pub(super) fn new() -> Self {
        Self::default()
    }

    /// Wrap a make-service so every connection it creates is tracked.
    pub(super) fn track<M>(&self, make_service: M) -> Tracked<M> {
        Tracked {
            inner: make_service,
            connections: self.clone(),
        }
    }

    /// Resolve once no connection's service, response future or response body
    /// remains. Await it only after the server stopped accepting, so no new
    /// token can appear.
    pub(super) async fn destroyed(&self) {
        self.0.close();
        self.0.wait().await;
    }

    /// `serving`'s return, then the destruction of everything its accepted
    /// connections held, which Axum counts finished before Hyper destroys it.
    /// The wait holds on an error return too, so dependent cleanup never runs
    /// beside such a destructor.
    pub(super) async fn after(
        &self,
        serving: impl IntoFuture<Output = std::io::Result<()>>,
    ) -> std::io::Result<()> {
        let served = serving.await;
        self.destroyed().await;
        served
    }

    fn token(&self) -> TaskTrackerToken {
        self.0.token()
    }
}

/// A make-service whose connections' services hold a token.
#[derive(Clone)]
pub(super) struct Tracked<M> {
    inner: M,
    connections: Connections,
}

impl<'a, L, M> Service<IncomingStream<'a, L>> for Tracked<M>
where
    L: Listener,
    M: Service<IncomingStream<'a, L>, Error = Infallible>,
{
    type Response = ConnectionService<M::Response>;
    type Error = Infallible;
    type Future = Connecting<M::Future>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Infallible>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, stream: IncomingStream<'a, L>) -> Self::Future {
        Connecting {
            inner: self.inner.call(stream),
            connections: self.connections.clone(),
        }
    }
}

pin_project! {
    /// Creating one connection's service.
    pub(super) struct Connecting<F> {
        #[pin]
        inner: F,
        connections: Connections,
    }
}

impl<F, S> Future for Connecting<F>
where
    F: Future<Output = Result<S, Infallible>>,
{
    type Output = Result<ConnectionService<S>, Infallible>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        let inner = ready!(this.inner.poll(cx))?;
        Poll::Ready(Ok(ConnectionService {
            inner,
            token: this.connections.token(),
        }))
    }
}

/// One connection's service. Hyper destroys it, with any response future in
/// flight, only when it destroys the connection. Its token is declared last,
/// so it is released only after the service it guards was destroyed.
#[derive(Clone)]
pub(super) struct ConnectionService<S> {
    inner: S,
    token: TaskTrackerToken,
}

impl<S> Service<Request> for ConnectionService<S>
where
    S: Service<Request, Response = Response, Error = Infallible>,
{
    type Response = Response;
    type Error = Infallible;
    type Future = Responding<S::Future>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Infallible>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, request: Request) -> Self::Future {
        // The request head decides, with the response, how Hyper frames it.
        let (method, version) = (request.method().clone(), request.version());
        Responding {
            inner: self.inner.call(request),
            method,
            version,
            token: Some(self.token.clone()),
        }
    }
}

pin_project! {
    /// One response in flight; its body inherits the token, which the future
    /// no longer needs once its response is ready.
    pub(super) struct Responding<F> {
        #[pin]
        inner: F,
        method: Method,
        version: Version,
        token: Option<TaskTrackerToken>,
    }
}

impl<F> Future for Responding<F>
where
    F: Future<Output = Result<Response, Infallible>>,
{
    type Output = Result<Response, Infallible>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        let response = ready!(this.inner.poll(cx))?;
        // The response Hyper writes, after every layer.
        let written = Written::of(this.method, *this.version, &response);
        let token = this.token.take();
        Poll::Ready(Ok(response.map(|inner| {
            Body::new(ConnectionBody {
                inner: Some(inner),
                written,
                _token: token,
            })
        })))
    }
}

/// A response body that keeps its connection's token until it is destroyed,
/// and makes its message's framing current for every call Hyper makes into
/// it, destruction included. It forwards every frame, end and size unchanged,
/// so framing is unaffected.
struct ConnectionBody {
    /// `None` only while it is being destroyed.
    inner: Option<Body>,
    written: Written,
    /// Declared last, so it is released after the body was destroyed.
    _token: Option<TaskTrackerToken>,
}

impl http_body::Body for ConnectionBody {
    type Data = Bytes;
    type Error = axum::Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, axum::Error>>> {
        let this = &mut *self;
        match this.inner.as_mut() {
            Some(inner) => this.written.writing(|| Pin::new(inner).poll_frame(cx)),
            None => Poll::Ready(None),
        }
    }

    fn is_end_stream(&self) -> bool {
        self.inner
            .as_ref()
            .is_none_or(|inner| self.written.writing(|| inner.is_end_stream()))
    }

    fn size_hint(&self) -> SizeHint {
        self.inner.as_ref().map_or_else(
            || SizeHint::with_exact(0),
            |inner| self.written.writing(|| inner.size_hint()),
        )
    }
}

impl Drop for ConnectionBody {
    fn drop(&mut self) {
        let inner = self.inner.take();
        self.written.writing(|| drop(inner));
    }
}

#[cfg(test)]
#[path = "completion_tests.rs"]
mod tests;
