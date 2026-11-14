//! A listener that reports when the server first reads request bytes, so a
//! test can start shutdown once a connection is no longer a fresh one instead
//! of guessing with a fixed sleep. Reads and writes pass through unchanged.

use axum::serve::Listener;
use std::{
    io,
    net::SocketAddr,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::{TcpListener, TcpStream},
    sync::Notify,
};

/// Wraps the bound listener; `read` is notified when a server read returns bytes.
pub(super) struct FirstRead {
    pub(super) inner: TcpListener,
    pub(super) read: Arc<Notify>,
}

impl Listener for FirstRead {
    type Io = Reported;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (Reported, SocketAddr) {
        let (inner, address) = <TcpListener as Listener>::accept(&mut self.inner).await;
        let read = self.read.clone();
        (Reported { inner, read }, address)
    }

    fn local_addr(&self) -> io::Result<SocketAddr> {
        self.inner.local_addr()
    }
}

pub(super) struct Reported {
    inner: TcpStream,
    read: Arc<Notify>,
}

impl AsyncRead for Reported {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let before = buf.filled().len();
        let polled = Pin::new(&mut self.inner).poll_read(cx, buf);
        if matches!(polled, Poll::Ready(Ok(()))) && buf.filled().len() > before {
            // The server keeps these bytes buffered once this poll returns;
            // a stored permit covers a waiter that arrives later.
            self.read.notify_one();
        }
        polled
    }
}

impl AsyncWrite for Reported {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write_vectored(cx, bufs)
    }

    fn is_write_vectored(&self) -> bool {
        self.inner.is_write_vectored()
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}
