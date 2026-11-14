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
//! every token has been released.

use axum::{
    body::{Body, Bytes},
    extract::Request,
    response::Response,
    serve::{IncomingStream, Listener},
};
use http_body::{Frame, SizeHint};
use pin_project_lite::pin_project;
use std::{
    convert::Infallible,
    future::{Future, poll_fn},
    pin::Pin,
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicUsize, Ordering},
    },
    task::{Context, Poll, Waker, ready},
};
use tower::Service;

/// The application state a server's accepted connections still hold.
///
/// Taking and releasing a token is one atomic operation; the lock is taken
/// only to store the waiting task and when the last token is released.
#[derive(Default)]
pub(super) struct Connections {
    tokens: AtomicUsize,
    idle: Mutex<Option<Waker>>,
}

impl Connections {
    pub(super) fn new() -> Arc<Self> {
        Arc::default()
    }

    /// Wrap a make-service so every connection it creates is tracked.
    pub(super) fn track<M>(self: &Arc<Self>, make_service: M) -> Tracked<M> {
        Tracked {
            inner: make_service,
            connections: self.clone(),
        }
    }

    /// Resolve once no connection's service, response future or response body
    /// remains. Await it only after the server stopped accepting, so no new
    /// token can appear.
    pub(super) async fn destroyed(&self) {
        poll_fn(|cx| {
            if self.tokens.load(Ordering::SeqCst) == 0 {
                return Poll::Ready(());
            }
            *self.idle.lock().unwrap_or_else(PoisonError::into_inner) = Some(cx.waker().clone());
            // A release that took no waker before it was stored is seen here.
            if self.tokens.load(Ordering::SeqCst) == 0 {
                Poll::Ready(())
            } else {
                Poll::Pending
            }
        })
        .await;
    }
}

/// One holder of a connection's application state. It is declared last in
/// each holder, so it is released only after what it guards was destroyed.
struct Token(Arc<Connections>);

impl Token {
    fn new(connections: &Arc<Connections>) -> Self {
        connections.tokens.fetch_add(1, Ordering::SeqCst);
        Self(connections.clone())
    }
}

impl Clone for Token {
    fn clone(&self) -> Self {
        Self::new(&self.0)
    }
}

impl Drop for Token {
    fn drop(&mut self) {
        if self.0.tokens.fetch_sub(1, Ordering::SeqCst) != 1 {
            return;
        }
        let idle = self
            .0
            .idle
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        // Woken outside the lock.
        if let Some(idle) = idle {
            idle.wake();
        }
    }
}

/// A make-service whose connections' services hold a token.
#[derive(Clone)]
pub(super) struct Tracked<M> {
    inner: M,
    connections: Arc<Connections>,
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
        connections: Arc<Connections>,
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
            token: Token::new(this.connections),
        }))
    }
}

/// One connection's service. Hyper destroys it, with any response future in
/// flight, only when it destroys the connection.
#[derive(Clone)]
pub(super) struct ConnectionService<S> {
    inner: S,
    token: Token,
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
        Responding {
            inner: self.inner.call(request),
            token: self.token.clone(),
        }
    }
}

pin_project! {
    /// One response in flight; its body inherits the token.
    pub(super) struct Responding<F> {
        #[pin]
        inner: F,
        token: Token,
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
        let token = this.token.clone();
        Poll::Ready(Ok(response.map(|inner| {
            Body::new(ConnectionBody {
                inner,
                _token: token,
            })
        })))
    }
}

/// A response body that keeps its connection's token until it is destroyed.
/// It forwards every frame, end and size unchanged, so framing is unaffected.
struct ConnectionBody {
    inner: Body,
    _token: Token,
}

impl http_body::Body for ConnectionBody {
    type Data = Bytes;
    type Error = axum::Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, axum::Error>>> {
        Pin::new(&mut self.inner).poll_frame(cx)
    }

    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }

    fn size_hint(&self) -> SizeHint {
        self.inner.size_hint()
    }
}

#[cfg(test)]
#[path = "completion_tests.rs"]
mod tests;
