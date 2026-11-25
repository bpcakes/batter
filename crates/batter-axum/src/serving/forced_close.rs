//! Connection I/O that fails once cooperative cancellation has had its share
//! of the supervisor's cancel allowance.
//!
//! Axum's graceful serving future waits for every connection task, and nothing
//! in Hyper ends a connection whose peer stopped reading or whose body stays
//! pending. Without this gate such a connection outlives the cancel allowance,
//! the supervisor aborts the server wrapper, and an abort skips dependent
//! cleanup. Failing the connection's own reads and writes instead lets Hyper
//! end it, so the wrapper returns after Axum observed every connection task
//! finish.

use crate::lock;
use atomic_waker::AtomicWaker;
use axum::serve::Listener;
use batter_core::lifecycle::ShutdownSignal;
use std::{
    collections::HashMap,
    fmt,
    future::Future,
    io::{self, IoSlice},
    pin::{Pin, pin},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll},
};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    time::Instant,
};

/// The one closing decision shared by a server's accepted connections.
#[derive(Default)]
pub(super) struct Gate {
    closed: AtomicBool,
    connections: Mutex<Connections>,
}

#[derive(Default)]
struct Connections {
    next: u64,
    pollers: HashMap<u64, Arc<Pollers>>,
}

/// The tasks that last polled one connection, by direction. An upgraded
/// connection may be split, so its halves can be polled by different tasks.
/// Recording one takes no lock: registering and waking an [`AtomicWaker`] are
/// read-modify-writes of one atomic, so either the close's wake sees the
/// registration or the registering poll's later read of the flag sees the
/// close.
#[derive(Default)]
struct Pollers {
    read: AtomicWaker,
    write: AtomicWaker,
}

impl Gate {
    pub(super) fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Wrap a listener so every connection it accepts observes this gate.
    pub(super) fn listener<L: Listener>(self: &Arc<Self>, inner: L) -> ForcedCloseListener<L> {
        ForcedCloseListener {
            inner,
            gate: self.clone(),
        }
    }

    fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    /// Fail all later I/O and wake every task that polled earlier I/O.
    ///
    /// The flag is published before the wakers are taken, and every poll
    /// rereads it after storing its waker, so each poll either fails or is
    /// woken by this close.
    fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
        let pollers: Vec<_> = lock(&self.connections).pollers.values().cloned().collect();
        // Waking application tasks happens outside the registry lock.
        for connection in pollers {
            connection.read.wake();
            connection.write.wake();
        }
    }

    /// Record the task polling one direction, then report whether the gate
    /// has closed.
    ///
    /// Every poll records its task, not only one the transport leaves
    /// pending. Hyper can wait on a response body alone without polling the
    /// transport again: after a ready read left bytes of a following request
    /// in its buffer, or after it closed the read side. That task's last I/O
    /// poll was ready, and the close must still wake it so its next pass,
    /// which flushes, fails.
    fn enter(&self, slot: &AtomicWaker, cx: &Context<'_>) -> bool {
        slot.register(cx.waker());
        self.is_closed()
    }
}

fn closed() -> io::Error {
    io::Error::new(
        io::ErrorKind::ConnectionAborted,
        "connection closed at forced shutdown",
    )
}

/// The process stop clock the gate follows: forced cancellation and the
/// supervisor's abort deadline, which only ever moves earlier. Production
/// passes the component's [`ShutdownSignal`]; a unit test drives the same
/// computation on a paused clock.
pub(super) trait StopClock {
    fn cancelled(&self) -> impl Future<Output = ()> + Send;
    fn abort_deadline(&self) -> Option<Instant>;
    fn abort_deadline_before(&self, known: Instant) -> impl Future<Output = Instant> + Send;
    /// When forced cancellation began, which can precede its observation.
    fn cancelled_at(&self) -> Option<Instant>;
}

impl StopClock for ShutdownSignal {
    fn cancelled(&self) -> impl Future<Output = ()> + Send {
        ShutdownSignal::cancelled(self)
    }

    fn cancelled_at(&self) -> Option<Instant> {
        ShutdownSignal::forced_cancellation_at(self)
    }

    fn abort_deadline(&self) -> Option<Instant> {
        ShutdownSignal::abort_deadline(self)
    }

    fn abort_deadline_before(&self, known: Instant) -> impl Future<Output = Instant> + Send {
        ShutdownSignal::abort_deadline_before(self, known)
    }
}

/// Serve until the native future returns, closing connection I/O part-way
/// through the cancel allowance.
///
/// Forced cancellation also cancels every admitted request context, so
/// handlers render their own interruption responses first. The gate closes
/// halfway between that instant, as the stop clock recorded it rather than
/// when this task observed it, and the supervisor's abort deadline, which
/// leaves the first half for those responses to be written and the second for
/// Axum to observe the closed connections finish. A component that reveals an
/// earlier stop time moves the abort deadline earlier, and the midpoint is
/// then recomputed from the same cancellation instant, closing at once if it
/// has passed while the abort deadline has not. Whenever the abort deadline
/// itself has passed, when cancellation is observed, as with a zero cancel
/// allowance, or after it moved earlier, the gate never closes and the wrapper
/// is aborted as before: closing then would only race that abort, on any
/// runtime flavour, and fail the interruption responses that detached
/// connections still write. A clock that reports no abort deadline, which the
/// supervisor's registration never does, leaves no known interval either, so
/// the gate stays open too.
pub(super) async fn serve<F, C>(serving: F, signal: C, gate: Arc<Gate>) -> io::Result<()>
where
    F: Future<Output = io::Result<()>>,
    C: StopClock,
{
    let mut serving = pin!(serving);
    let closing = async {
        signal.cancelled().await;
        // A busy runtime can poll this task late; the halves are measured
        // from when cancellation began.
        let cancelled = signal.cancelled_at().unwrap_or_else(Instant::now);
        while let Some(abort) = signal.abort_deadline() {
            let close = cancelled + abort.saturating_duration_since(cancelled) / 2;
            tokio::select! {
                biased;
                () = tokio::time::sleep_until(close) => break,
                _ = signal.abort_deadline_before(abort) => {}
            }
        }
        // No interval left, at cancellation or because the deadline moved into
        // the past, even while a stale midpoint won the race above: the wrapper
        // abort decides, and closing would only race it. Nor is there a known
        // interval without a deadline, so cancelled handlers keep their share.
        if signal
            .abort_deadline()
            .is_none_or(|abort| abort <= Instant::now())
        {
            std::future::pending::<()>().await;
        }
        gate.close();
    };
    tokio::select! {
        biased;
        result = &mut serving => return result,
        () = closing => {}
    }
    serving.await
}

pub(super) struct ForcedCloseListener<L> {
    inner: L,
    gate: Arc<Gate>,
}

impl<L: Listener> Listener for ForcedCloseListener<L> {
    type Io = ForcedCloseIo<L::Io>;
    type Addr = L::Addr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        let (io, address) = self.inner.accept().await;
        (ForcedCloseIo::new(io, self.gate.clone()), address)
    }

    fn local_addr(&self) -> io::Result<Self::Addr> {
        self.inner.local_addr()
    }
}

/// One connection a registered server accepted: the listener's own I/O behind
/// the forced connection close.
///
/// Every server registered through `AssembledHttp::register_in`,
/// `AssembledHttp::register_with_connect_info_in` or the
/// [`register_http`](crate::low_level::register_http) helpers gives Hyper this
/// type in place of the listener's `Io`, so that the connection's reads and
/// writes can be failed at forced shutdown. Ordinary handlers never see it.
/// It matters only to code that recovers the connection after an HTTP/1
/// upgrade by its concrete type: `I` is the listener's `Io`, such as
/// [`tokio::net::TcpStream`] for a [`tokio::net::TcpListener`], and the type
/// to name in `hyper_util::server::conn::auto::upgrade::downcast` is
/// `hyper_util::rt::TokioIo<ForcedCloseIo<I>>`. That helper is required:
/// the connection builder Axum uses keeps the I/O inside a private wrapper
/// of hyper-util's, so `hyper::upgrade::Upgraded::downcast` with the same
/// type returns `Err`.
///
/// Migration: a downcast of an upgraded connection to the listener's own
/// type, such as
/// `hyper_util::server::conn::auto::upgrade::downcast::<TokioIo<TcpStream>>`,
/// matched while a server registered here handed Hyper the listener's `Io`
/// unchanged. It now always returns `Err`; name
/// `TokioIo<ForcedCloseIo<TcpStream>>` instead, then call
/// [`into_inner`](Self::into_inner) for the socket. `Upgraded` used as a
/// plain byte stream, as Axum's WebSocket extractor uses it, needs no change.
///
/// Hyper may have read past the upgrade request. Those bytes are the start of
/// the upgraded protocol, the socket no longer has them, and the downcast
/// returns them beside the I/O: keep them with the socket and consume them
/// before reading from it.
///
/// ```
/// use axum::body::Bytes;
/// use batter_axum::low_level::ForcedCloseIo;
/// use hyper_util::{rt::TokioIo, server::conn::auto::upgrade::downcast};
/// use tokio::{io::AsyncReadExt, net::TcpStream};
///
/// // `upgraded` is what `hyper::upgrade::on` resolved to for a request served
/// // over a `TcpListener`. The socket comes back with the bytes Hyper
/// // already took from it.
/// fn recover(upgraded: hyper::upgrade::Upgraded) -> Option<(TcpStream, Bytes)> {
///     let parts = downcast::<TokioIo<ForcedCloseIo<TcpStream>>>(upgraded).ok()?;
///     Some((parts.io.into_inner().into_inner(), parts.read_buf))
/// }
///
/// // The upgraded protocol's opening bytes, at least `wanted` of them unless
/// // the peer closes first: the buffered ones, then what the socket yields.
/// async fn opening(
///     upgraded: hyper::upgrade::Upgraded,
///     wanted: usize,
/// ) -> std::io::Result<(TcpStream, Vec<u8>)> {
///     let (mut socket, buffered) =
///         recover(upgraded).ok_or(std::io::ErrorKind::Unsupported)?;
///     let mut protocol = buffered.to_vec();
///     while protocol.len() < wanted && socket.read_buf(&mut protocol).await? != 0 {}
///     Ok((socket, protocol))
/// }
/// # let _ = opening;
/// ```
///
/// Caller obligations: an upgraded connection belongs to the application. The
/// task that holds it is never joined by the registered server, and no
/// request deadline, body bound or admission covers it. Reads and writes
/// polled through this wrapper still fail at the forced close, but only if
/// one happens: there is none with a zero cancel allowance, and it is
/// skipped once the registered server has returned from its graceful
/// shutdown and the connections it tracks were destroyed, which a
/// connection upgraded over HTTP/1 does not prolong.
/// And only those reads and writes fail: I/O done on the listener's own
/// type, through [`get_ref`](Self::get_ref) or after
/// [`into_inner`](Self::into_inner), never meets the close. The application
/// must end such a connection from its own shutdown observation.
pub struct ForcedCloseIo<I> {
    inner: I,
    entry: Entry,
}

/// One connection's place in its gate's registry, removed when the connection
/// is destroyed or its I/O is recovered.
struct Entry {
    gate: Arc<Gate>,
    id: u64,
    pollers: Arc<Pollers>,
}

impl Entry {
    /// Record the task polling for reads, then report whether the gate closed.
    fn reading(&self, cx: &Context<'_>) -> bool {
        self.gate.enter(&self.pollers.read, cx)
    }

    /// Record the task polling for writes, then report whether the gate closed.
    fn writing(&self, cx: &Context<'_>) -> bool {
        self.gate.enter(&self.pollers.write, cx)
    }
}

impl Drop for Entry {
    fn drop(&mut self) {
        lock(&self.gate.connections).pollers.remove(&self.id);
    }
}

impl<I> ForcedCloseIo<I> {
    fn new(inner: I, gate: Arc<Gate>) -> Self {
        let pollers = Arc::new(Pollers::default());
        let id = {
            let mut connections = lock(&gate.connections);
            let id = connections.next;
            connections.next += 1;
            connections.pollers.insert(id, pollers.clone());
            id
        };
        Self {
            inner,
            entry: Entry { gate, id, pollers },
        }
    }

    /// The listener's own I/O, for reading what it exposes, such as a
    /// socket's addresses or a TLS session's negotiated parameters.
    ///
    /// A shared reference is not read-only access. Many I/O types read and
    /// write through one: a [`tokio::net::TcpStream`] offers `readable`,
    /// `try_read` and `try_write` on `&self`. An operation done that way
    /// bypasses this wrapper: it is not failed at forced shutdown and
    /// registers no waker with the forced close, so the close does not end a
    /// wait in it. The connection itself stays registered, unlike after
    /// [`into_inner`](Self::into_inner): reads and writes through the wrapper
    /// still fail at the close, and a task whose last poll went through the
    /// wrapper is still woken by it, which does not fail a direct operation
    /// that task is then waiting in. Do I/O only through the wrapper's own
    /// `AsyncRead` and `AsyncWrite`, or take on ending the connection from
    /// the application's own shutdown observation.
    pub fn get_ref(&self) -> &I {
        &self.inner
    }

    /// Recover the listener's own I/O, leaving the forced connection close.
    ///
    /// The returned I/O is never failed at forced shutdown, and this
    /// connection is removed from those a later close wakes. A close already
    /// under way when this is called may still wake a task that polled the
    /// wrapper earlier; that wake fails nothing. Use it only for a connection
    /// the application owns after an upgrade, and end that connection from
    /// the application's own shutdown observation.
    pub fn into_inner(self) -> I {
        // The registration goes first, while the I/O is still an owned
        // local, so nothing runs after it is on its way out.
        let Self { inner, entry } = self;
        drop(entry);
        inner
    }
}

impl<I> fmt::Debug for ForcedCloseIo<I> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ForcedCloseIo")
            .finish_non_exhaustive()
    }
}

impl<I: AsyncRead + Unpin> AsyncRead for ForcedCloseIo<I> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if self.entry.reading(cx) {
            return Poll::Ready(Err(closed()));
        }
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl<I: AsyncWrite + Unpin> AsyncWrite for ForcedCloseIo<I> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        if self.entry.writing(cx) {
            return Poll::Ready(Err(closed()));
        }
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        if self.entry.writing(cx) {
            return Poll::Ready(Err(closed()));
        }
        Pin::new(&mut self.inner).poll_write_vectored(cx, bufs)
    }

    fn is_write_vectored(&self) -> bool {
        self.inner.is_write_vectored()
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if self.entry.writing(cx) {
            return Poll::Ready(Err(closed()));
        }
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    // A closed gate also fails shutdown: a transport that must write to finish
    // closing would otherwise wait on the same stalled peer.
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if self.entry.writing(cx) {
            return Poll::Ready(Err(closed()));
        }
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

#[cfg(test)]
#[path = "forced_close_tests.rs"]
mod tests;
