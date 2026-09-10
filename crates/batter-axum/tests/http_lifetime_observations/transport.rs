//! Forward native socket operations unchanged, acknowledging actual handle drop.
use super::resource::Events;
use axum::serve::Listener;
use std::{
    io,
    net::SocketAddr,
    pin::Pin,
    task::{Context, Poll},
};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::{TcpListener, TcpStream},
};

pub struct TrackedListener {
    pub inner: TcpListener,
    pub events: Events,
}
impl Listener for TrackedListener {
    type Io = TrackedSocket;
    type Addr = SocketAddr;
    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        let (socket, address) = <TcpListener as Listener>::accept(&mut self.inner).await;
        self.events.record("socket-accepted");
        (
            TrackedSocket {
                inner: Some(socket),
                events: self.events.clone(),
            },
            address,
        )
    }
    fn local_addr(&self) -> io::Result<SocketAddr> {
        self.inner.local_addr()
    }
}

pub struct TrackedSocket {
    inner: Option<TcpStream>,
    events: Events,
}
impl AsyncRead for TrackedSocket {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(self.inner.as_mut().unwrap()).poll_read(cx, buf)
    }
}
impl AsyncWrite for TrackedSocket {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(self.inner.as_mut().unwrap()).poll_write(cx, buf)
    }
    fn is_write_vectored(&self) -> bool {
        self.inner.as_ref().unwrap().is_write_vectored()
    }
    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        Pin::new(self.inner.as_mut().unwrap()).poll_write_vectored(cx, bufs)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(self.inner.as_mut().unwrap()).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(self.inner.as_mut().unwrap()).poll_shutdown(cx)
    }
}
impl Drop for TrackedSocket {
    fn drop(&mut self) {
        drop(self.inner.take());
        self.events.record("server-socket-drop");
    }
}
