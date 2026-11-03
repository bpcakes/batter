//! The library-built HTTP client profile.

use crate::{
    limits::ConnectTimeout,
    origin::{Origin, Scheme},
    request::RequestDraft,
};
use http::Method;
use rustls_platform_verifier::BuilderVerifierExt;
use std::{fmt, sync::Arc, time::Duration};

/// The client refuses to reuse a connection that has been idle for longer than
/// this, which is shorter than common server keep-alive timeouts (five seconds
/// is a frequent default). A request is then rarely written to a connection the
/// server is already closing, which would be reported as
/// [`crate::Exchange::Unknown`].
const POOL_IDLE_TIMEOUT: Duration = Duration::from_secs(4);

/// A client for one provider origin, configured only by this package.
///
/// Every client:
/// - follows no redirects: a 3xx is returned as a response (tested);
/// - offers only `http/1.1` in ALPN, even when another crate enables reqwest's
///   HTTP/2 support (tested);
/// - turns off reqwest's response decompression, so the body limit counts
///   message-body bytes as received (tested for gzip; Brotli, zstd and deflate
///   support is not compiled in this workspace, so those switches are
///   defence in depth);
/// - ignores proxy environment variables (tested with `HTTP_PROXY` for a
///   plain-HTTP origin; `HTTPS_PROXY` and `ALL_PROXY` for HTTPS origins are not
///   tested);
/// - never reuses a connection idle for more than four seconds (tested);
/// - uses rustls with the *ring* provider passed explicitly, TLS 1.3 or 1.2
///   only, and the operating system's certificate verifier for HTTPS origins
///   (the offered versions and ALPN are tested). It neither reads nor installs a
///   process-default crypto provider (tested: none is installed after building
///   the client and sending a ClientHello; no test completes a handshake) and
///   adds no aws-lc-rs;
/// - allows plain HTTP only for [`Origin::loopback`], whose client never reads
///   the host certificate store (tested on Linux).
///
/// Using this package compiles rustls's `ring` feature and reqwest 0.13's
/// `rustls-no-provider` feature into the application. If the application's
/// graph also compiles aws-lc-rs into rustls, crates that call
/// `rustls::ClientConfig::builder()` without a provider panic; and a
/// `reqwest::Client::new()` elsewhere panics without an installed provider when
/// the graph's reqwest 0.13 has only `rustls-no-provider` (neither its `rustls`
/// nor its `native-tls` feature). In either case the application root installs
/// a process default with `rustls::crypto::CryptoProvider::install_default`.
///
/// Also set as defence in depth, not observable while redirects are off and
/// ALPN offers only HTTP/1.1: `http1_only()`, `https_only` for an HTTPS origin,
/// reqwest's own retries disabled and no `Referer`.
///
/// A built client cannot be reconfigured. Cloning shares one connection pool.
/// `Debug` shows no configuration.
#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    origin: Origin,
}

impl Client {
    /// Build a client for `origin`. Nothing connects until a request is sent.
    ///
    /// For an HTTPS origin on Linux, construction loads the host certificate
    /// store, which follows `SSL_CERT_FILE` and `SSL_CERT_DIR`, and returns
    /// [`ClientError::Build`] when it is empty (tested). On macOS the system
    /// verifier is consulted during each handshake instead (source-inspected). Construction is
    /// not free (about 28 ms was measured once in a Linux debug build; not a
    /// contract), so build one client per origin and clone it, and take a
    /// request's deadline after preparing it.
    pub fn new(origin: Origin, connect_timeout: ConnectTimeout) -> Result<Self, ClientError> {
        let http = reqwest::Client::builder()
            .tls_backend_preconfigured(tls_config(origin.scheme())?)
            .redirect(reqwest::redirect::Policy::none())
            .referer(false)
            .retry(reqwest::retry::never())
            .no_proxy()
            .http1_only()
            .no_gzip()
            .no_brotli()
            .no_zstd()
            .no_deflate()
            .no_hickory_dns()
            .connect_timeout(connect_timeout.get())
            .pool_idle_timeout(POOL_IDLE_TIMEOUT)
            .https_only(origin.scheme() == Scheme::Https)
            .build()
            .map_err(|_| ClientError::Build)?;
        Ok(Self { http, origin })
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
            self.http.clone(),
            self.origin.url().clone(),
            method,
            path.into_iter().map(str::to_owned).collect(),
        )
    }
}

/// The TLS configuration handed to reqwest, which uses it as given.
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
