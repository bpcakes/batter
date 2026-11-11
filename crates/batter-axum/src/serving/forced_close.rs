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

use axum::serve::Listener;
use batter_core::lifecycle::ShutdownSignal;
use std::{
    collections::HashMap,
    future::Future,
    io::{self, IoSlice},
    pin::{Pin, pin},
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll, Waker},
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
    parked: HashMap<u64, Arc<Parked>>,
}

/// The tasks one connection last left waiting, by direction. An upgraded
/// connection may be split, so its halves can park different tasks.
#[derive(Default)]
struct Parked {
    read: Mutex<Option<Waker>>,
    write: Mutex<Option<Waker>>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
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

    /// Fail all later I/O and wake every task parked in earlier I/O.
    ///
    /// The flag is published before the wakers are taken, and a parking poll
    /// rereads it after storing its waker, so no poll can stay parked.
    fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
        let parked: Vec<_> = lock(&self.connections).parked.values().cloned().collect();
        // Waking application tasks happens outside the registry lock.
        for connection in parked {
            for slot in [&connection.read, &connection.write] {
                let waker = lock(slot).take();
                if let Some(waker) = waker {
                    waker.wake();
                }
            }
        }
    }

    fn park<T>(&self, slot: &Mutex<Option<Waker>>, cx: &Context<'_>) -> Poll<io::Result<T>> {
        {
            let mut parked = lock(slot);
            if !parked
                .as_ref()
                .is_some_and(|waker| waker.will_wake(cx.waker()))
            {
                *parked = Some(cx.waker().clone());
            }
        }
        if self.is_closed() {
            Poll::Ready(Err(closed()))
        } else {
            Poll::Pending
        }
    }
}

fn closed() -> io::Error {
    io::Error::new(
        io::ErrorKind::ConnectionAborted,
        "connection closed at forced shutdown",
    )
}

/// Serve until the native future returns, closing connection I/O part-way
/// through the cancel allowance.
///
/// Forced cancellation also cancels every admitted request context, so
/// handlers render their own interruption responses first. The gate closes
/// halfway between that instant and the supervisor's abort deadline, which
/// leaves the first half for those responses to be written and the second for
/// Axum to observe the closed connections finish. A zero cancel allowance
/// leaves no interval, and the wrapper is then aborted as before.
pub(super) async fn serve<F>(serving: F, signal: ShutdownSignal, gate: Arc<Gate>) -> io::Result<()>
where
    F: Future<Output = io::Result<()>>,
{
    let mut serving = pin!(serving);
    let closing = async {
        signal.cancelled().await;
        if let Some(abort) = signal.abort_deadline() {
            let now = Instant::now();
            tokio::time::sleep_until(now + abort.saturating_duration_since(now) / 2).await;
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

pub(super) struct ForcedCloseIo<I> {
    inner: I,
    gate: Arc<Gate>,
    id: u64,
    parked: Arc<Parked>,
}

impl<I> ForcedCloseIo<I> {
    fn new(inner: I, gate: Arc<Gate>) -> Self {
        let parked = Arc::new(Parked::default());
        let id = {
            let mut connections = lock(&gate.connections);
            let id = connections.next;
            connections.next += 1;
            connections.parked.insert(id, parked.clone());
            id
        };
        Self {
            inner,
            gate,
            id,
            parked,
        }
    }
}

impl<I> Drop for ForcedCloseIo<I> {
    fn drop(&mut self) {
        lock(&self.gate.connections).parked.remove(&self.id);
    }
}

impl<I: AsyncRead + Unpin> AsyncRead for ForcedCloseIo<I> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if self.gate.is_closed() {
            return Poll::Ready(Err(closed()));
        }
        match Pin::new(&mut self.inner).poll_read(cx, buf) {
            Poll::Pending => self.gate.park(&self.parked.read, cx),
            ready => ready,
        }
    }
}

impl<I: AsyncWrite + Unpin> AsyncWrite for ForcedCloseIo<I> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        if self.gate.is_closed() {
            return Poll::Ready(Err(closed()));
        }
        match Pin::new(&mut self.inner).poll_write(cx, buf) {
            Poll::Pending => self.gate.park(&self.parked.write, cx),
            ready => ready,
        }
    }

    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        if self.gate.is_closed() {
            return Poll::Ready(Err(closed()));
        }
        match Pin::new(&mut self.inner).poll_write_vectored(cx, bufs) {
            Poll::Pending => self.gate.park(&self.parked.write, cx),
            ready => ready,
        }
    }

    fn is_write_vectored(&self) -> bool {
        self.inner.is_write_vectored()
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if self.gate.is_closed() {
            return Poll::Ready(Err(closed()));
        }
        match Pin::new(&mut self.inner).poll_flush(cx) {
            Poll::Pending => self.gate.park(&self.parked.write, cx),
            ready => ready,
        }
    }

    // A closed gate also fails shutdown: a transport that must write to finish
    // closing would otherwise wait on the same stalled peer.
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if self.gate.is_closed() {
            return Poll::Ready(Err(closed()));
        }
        match Pin::new(&mut self.inner).poll_shutdown(cx) {
            Poll::Pending => self.gate.park(&self.parked.write, cx),
            ready => ready,
        }
    }
}

#[cfg(test)]
#[path = "forced_close_tests.rs"]
mod tests;
