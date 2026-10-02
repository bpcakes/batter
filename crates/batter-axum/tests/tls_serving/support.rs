//! Test-owned transport policy for the generic listener contract.
//!
//! Everything here is the application's responsibility in a real deployment:
//! the certificate, the crypto provider, the accept loop and the handshake. The
//! adapter under test contributes only registration, acknowledgement, drain and
//! cleanup, so none of this belongs in `batter-axum` itself.

use axum::serve::Listener;
use rustls::{
    ClientConfig, RootCertStore, ServerConfig,
    pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer, ServerName},
};
use std::{io, net::SocketAddr, sync::Arc, time::Duration};
use tokio::{
    net::{TcpListener, TcpStream},
    sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel},
};
use tokio_rustls::{TlsAcceptor, TlsConnector, client, server};

pub const WAIT: Duration = Duration::from_secs(5);
/// Generated for an operator-chosen name; the client verifies exactly it.
pub const SERVER_NAME: &str = "listener.test";

/// What the listener observed, so a test can await accept progress instead of
/// polling for it.
#[derive(Debug, Eq, PartialEq)]
pub enum Accepting {
    /// A transport connection was accepted and its handshake has begun.
    HandshakeStarted,
    /// An accept or handshake was destroyed before it produced a connection.
    PendingReleased,
}

/// A generated certificate plus the two rustls configurations that trust it.
pub fn certificates() -> (ServerConfig, ClientConfig) {
    let issued = rcgen::generate_simple_self_signed(vec![SERVER_NAME.to_owned()]).unwrap();
    let certificate = issued.cert.der().clone();
    let key = PrivatePkcs8KeyDer::from(issued.signing_key.serialize_der());
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let server = ServerConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(vec![certificate.clone()], PrivateKeyDer::Pkcs8(key))
        .unwrap();
    let mut roots = RootCertStore::empty();
    roots.add(certificate).unwrap();
    let client = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(roots)
        .with_no_client_auth();
    (server, client)
}

/// An application-owned TLS listener. It completes each handshake inside
/// `accept`, where Axum polls it, and reports the accepted transport peer.
pub struct TlsListener {
    transport: TcpListener,
    acceptor: TlsAcceptor,
    accepting: UnboundedSender<Accepting>,
}

impl TlsListener {
    /// Bind a loopback TLS listener and return the channel of its observations.
    pub async fn bind(config: ServerConfig) -> (Self, UnboundedReceiver<Accepting>) {
        let transport = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let (accepting, observations) = unbounded_channel();
        let listener = Self {
            transport,
            acceptor: TlsAcceptor::from(Arc::new(config)),
            accepting,
        };
        (listener, observations)
    }

    pub fn address(&self) -> SocketAddr {
        self.transport.local_addr().unwrap()
    }
}

/// Reports destruction of an accept or handshake that never produced a
/// connection. Resolving the attempt disarms it.
struct PendingAccept {
    accepting: UnboundedSender<Accepting>,
    resolved: bool,
}

impl Drop for PendingAccept {
    fn drop(&mut self) {
        if !self.resolved {
            let _ = self.accepting.send(Accepting::PendingReleased);
        }
    }
}

impl Listener for TlsListener {
    type Io = server::TlsStream<TcpStream>;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        loop {
            let mut pending = PendingAccept {
                accepting: self.accepting.clone(),
                resolved: false,
            };
            // Axum's trait forbids returning accept errors, so this listener
            // owns its own retry policy, exactly as a deployed one would.
            let Ok((transport, peer)) = self.transport.accept().await else {
                pending.resolved = true;
                tokio::time::sleep(Duration::from_millis(10)).await;
                continue;
            };
            let _ = self.accepting.send(Accepting::HandshakeStarted);
            match self.acceptor.accept(transport).await {
                Ok(connection) => {
                    pending.resolved = true;
                    return (connection, peer);
                }
                Err(_) => {
                    // A rejected handshake is application policy, not a
                    // lifecycle event: drop the connection and keep serving.
                    pending.resolved = true;
                }
            }
        }
    }

    fn local_addr(&self) -> io::Result<SocketAddr> {
        self.transport.local_addr()
    }
}

/// Complete a client handshake against the generated certificate.
pub async fn connect(config: &ClientConfig, address: SocketAddr) -> client::TlsStream<TcpStream> {
    let transport = TcpStream::connect(address).await.unwrap();
    TlsConnector::from(Arc::new(config.clone()))
        .connect(ServerName::try_from(SERVER_NAME).unwrap(), transport)
        .await
        .unwrap()
}

/// The transport address the client itself owns, as an oracle for peer
/// metadata that never comes from the server.
pub fn client_address(stream: &client::TlsStream<TcpStream>) -> SocketAddr {
    stream.get_ref().0.local_addr().unwrap()
}
