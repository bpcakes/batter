//! The library-built HTTP client profile.

use crate::{
    limits::ConnectTimeout,
    origin::{Origin, Scheme},
    request::RequestDraft,
};
use http::Method;
use rustls_platform_verifier::BuilderVerifierExt;
use std::{fmt, sync::Arc, time::Duration};

/// A client for one provider origin, configured only by this package.
///
/// Each exchange opens a fresh TCP connection and, for HTTPS, a TLS
/// handshake. Its HTTP/1 connection is driven inside the exchange future, never
/// spawned. There is no connection pool, replay, redirect, proxy lookup,
/// automatic content decoding or HTTP/2 negotiation.
///
/// Hostname setup tries the first resolved address family first and starts the
/// other after at most 300 ms (or half the remaining setup budget when shorter).
/// Addresses within a family share its remaining budget. At most two TCP
/// attempts run together, in this future; losing sockets are dropped before TLS.
///
/// TLS uses an explicit *ring* provider, TLS 1.3 and 1.2 only, ALPN `http/1.1`,
/// and the operating system's certificate verifier for HTTPS. Loopback HTTP
/// clients use an empty root store and never read the host certificate store.
/// The ClientHello and empty-store behavior have executable tests.
/// No process-default crypto provider is read or installed. Applications whose
/// other dependencies enable both rustls providers must configure those
/// dependencies or install their chosen default at the application root.
///
/// A built client cannot be reconfigured. Cloning shares TLS configuration,
/// never connections. `Debug` shows no configuration.
#[derive(Clone)]
pub struct Client {
    pub(crate) tls: Arc<rustls::ClientConfig>,
    pub(crate) connect_timeout: Duration,
    pub(crate) origin: Origin,
}

impl Client {
    /// Build a client for `origin`. Nothing connects until a request is sent.
    ///
    /// For an HTTPS origin on Linux, construction loads the host certificate
    /// store, which follows `SSL_CERT_FILE` and `SSL_CERT_DIR`, and returns
    /// [`ClientError::Build`] when it is empty (tested). On macOS the system
    /// verifier is consulted during each handshake instead (source-inspected).
    /// Build one client per origin, clone its configuration, and take a
    /// request's deadline after preparing it.
    pub fn new(origin: Origin, connect_timeout: ConnectTimeout) -> Result<Self, ClientError> {
        Ok(Self {
            tls: Arc::new(tls_config(origin.scheme())?),
            connect_timeout: connect_timeout.get(),
            origin,
        })
    }

    /// Start a request to the path below this client's origin made of `path`.
    ///
    /// Each segment is percent-encoded, so `/`, `?` and `#` inside a segment stay
    /// part of it: pass `["v1", "effects"]`, not `["v1/effects"]`, which would
    /// be sent as one encoded segment. [`RequestDraft::prepare`] refuses empty,
    /// `.` and `..` segments, which URL normalization would otherwise drop
    /// silently, segments whose part before a `;` is empty, `.` or `..`, which
    /// servers that strip `;` parameters would treat the same way, and segments
    /// containing a tab, line feed or carriage return, which URL parsing
    /// deletes. An empty `path` targets the origin root.
    pub fn request<'a, I>(&self, method: Method, path: I) -> RequestDraft
    where
        I: IntoIterator<Item = &'a str>,
    {
        RequestDraft::new(
            self.clone(),
            self.origin.url().clone(),
            method,
            path.into_iter().map(str::to_owned).collect(),
        )
    }
}

/// The TLS configuration used directly by the owned connection.
fn tls_config(scheme: Scheme) -> Result<rustls::ClientConfig, ClientError> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let builder = rustls::ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13, &rustls::version::TLS12])
        .map_err(|_| ClientError::Build)?;
    let mut config = match scheme {
        Scheme::Https => builder
            .with_platform_verifier()
            .map_err(|_| ClientError::Build)?
            .with_no_client_auth(),
        // Plain HTTP only, and every request URL comes from this origin, so no
        // TLS session can start: trusting nothing keeps the host store unread.
        Scheme::LoopbackHttp => builder
            .with_root_certificates(rustls::RootCertStore::empty())
            .with_no_client_auth(),
    };
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(config)
}

impl fmt::Debug for Client {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Client { .. }")
    }
}

/// Why a client could not be built. The message carries no configuration.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ClientError {
    /// The HTTP client could not be initialized, typically because the
    /// operating system's certificate store could not be loaded for an HTTPS
    /// origin.
    #[error("the outbound HTTP client could not be built")]
    Build,
}
