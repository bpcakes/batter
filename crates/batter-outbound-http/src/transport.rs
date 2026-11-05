//! Owned TCP/TLS transport. No connection driver or writer is spawned.

use crate::{client::Client, exchange::StartRefusal, origin::Scheme};
use std::{
    io::{self, IoSlice},
    net::SocketAddr,
    pin::Pin,
    task::{Context, Poll},
};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::TcpStream,
    time::Instant,
};
use tokio_rustls::TlsConnector;

mod dial;

/// Shared by the handoff gate and both sides of TLS. A TLS read can flush
/// encrypted records, so guarding only the plaintext writer is insufficient.
pub(crate) struct Gate<F> {
    pub(crate) deadline: Instant,
    pub(crate) is_cancelled: F,
}

impl<F: Fn() -> bool> Gate<F> {
    pub(crate) fn stopped(&self) -> Option<StartRefusal> {
        if (self.is_cancelled)() {
            Some(StartRefusal::Cancelled)
        } else if Instant::now() >= self.deadline {
            Some(StartRefusal::DeadlineTooClose)
        } else {
            None
        }
    }
}

pub(crate) trait Io: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Io for T {}

pub(crate) async fn connect<'a, F: Fn() -> bool + Sync>(
    client: &Client,
    gate: &'a Gate<F>,
    connect_deadline: Instant,
) -> io::Result<Box<dyn Io + 'a>> {
    let url = client.origin.url();
    let port = url
        .port_or_known_default()
        .ok_or(io::ErrorKind::InvalidInput)?;
    let addresses = match url.host().ok_or(io::ErrorKind::InvalidInput)? {
        url::Host::Ipv4(ip) => vec![SocketAddr::new(ip.into(), port)],
        url::Host::Ipv6(ip) => vec![SocketAddr::new(ip.into(), port)],
        url::Host::Domain(host) => tokio::net::lookup_host((host, port)).await?.collect(),
    };
    let tcp = dial::connect(
        &client.origin,
        addresses,
        connect_deadline.min(gate.deadline),
        TcpStream::connect,
    )
    .await?;
    tcp.set_nodelay(true)?;
    match client.origin.scheme() {
        Scheme::LoopbackHttp => Ok(Box::new(Guarded { inner: tcp, gate })),
        Scheme::Https => {
            let host = match url.host().ok_or(io::ErrorKind::InvalidInput)? {
                url::Host::Ipv4(ip) => ip.to_string(),
                url::Host::Ipv6(ip) => ip.to_string(),
                url::Host::Domain(host) => host.to_owned(),
            };
            let name = rustls::pki_types::ServerName::try_from(host)
                .map_err(|_| io::ErrorKind::InvalidInput)?;
            Ok(Box::new(secure(tcp, client.tls.clone(), name, gate).await?))
        }
    }
}

type Secured<'a, I, F> = Guarded<'a, tokio_rustls::client::TlsStream<Guarded<'a, I, F>>, F>;

/// Keep both guards in the same assembly used by production and TLS fault tests.
async fn secure<'a, I, F>(
    io: I,
    config: std::sync::Arc<rustls::ClientConfig>,
    name: rustls::pki_types::ServerName<'static>,
    gate: &'a Gate<F>,
) -> io::Result<Secured<'a, I, F>>
where
    I: AsyncRead + AsyncWrite + Unpin,
    F: Fn() -> bool,
{
    let tcp = Guarded { inner: io, gate };
    let tls = TlsConnector::from(config).connect(name, tcp).await?;
    // TLS may accept plaintext without polling TCP, and a TLS read may flush
    // ciphertext. Both sides must independently check the same exchange gate.
    Ok(Guarded { inner: tls, gate })
}

struct Guarded<'a, I, F> {
    inner: I,
    gate: &'a Gate<F>,
}

impl<I: AsyncRead + Unpin, F> AsyncRead for Guarded<'_, I, F> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_read(cx, buf)
    }
}

impl<I: AsyncWrite + Unpin, F: Fn() -> bool> AsyncWrite for Guarded<'_, I, F> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        if this.gate.stopped().is_some() {
            return Poll::Pending;
        }
        Pin::new(&mut this.inner).poll_write(cx, buf)
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        if this.gate.stopped().is_some() {
            return Poll::Pending;
        }
        Pin::new(&mut this.inner).poll_write_vectored(cx, bufs)
    }

    fn is_write_vectored(&self) -> bool {
        self.inner.is_write_vectored()
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if this.gate.stopped().is_some() {
            return Poll::Pending;
        }
        Pin::new(&mut this.inner).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if this.gate.stopped().is_some() {
            return Poll::Pending;
        }
        Pin::new(&mut this.inner).poll_shutdown(cx)
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod tls_tests;

#[cfg(test)]
mod tls_write_tests;
