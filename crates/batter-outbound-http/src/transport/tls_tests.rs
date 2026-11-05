//! Real TLS handshakes through the production transport, including a readable
//! response after expiry. The trusting control changes only its private verifier.

use crate::{Client, ConnectFailure, ConnectTimeout, Exchange, Origin, ResponseLimit};
use std::{sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    time::Instant,
};

#[tokio::test(flavor = "current_thread")]
async fn owned_tls_verifies_certificates_and_keeps_readable_replies_after_expiry() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let rcgen::CertifiedKey { cert, signing_key } =
            rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        for version in [&rustls::version::TLS12, &rustls::version::TLS13] {
            let provider = Arc::new(rustls::crypto::ring::default_provider());
            let mut server = rustls::ServerConfig::builder_with_provider(provider.clone())
                .with_protocol_versions(&[version]).unwrap()
                .with_no_client_auth()
                .with_single_cert(vec![cert.der().clone()],
                    rustls::pki_types::PrivatePkcs8KeyDer::from(signing_key.serialize_der()).into()).unwrap();
            server.alpn_protocols = vec![b"http/1.1".to_vec()];
            let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(server));
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let origin = Origin::https(&format!("https://localhost:{}", listener.local_addr().unwrap().port())).unwrap();
            let (written, ready) = tokio::sync::oneshot::channel();
            let peer = tokio::spawn(async move {
                for trusted in [false, true, true] {
                    let (tcp, _) = listener.accept().await.unwrap();
                    let stream = acceptor.accept(tcp).await;
                    if !trusted {
                        assert!(stream.is_err(), "untrusted certificate was accepted");
                        continue;
                    }
                    let mut stream = stream.unwrap();
                    assert_eq!(stream.get_ref().1.alpn_protocol(), Some(b"http/1.1".as_slice()));
                    let mut head = Vec::new();
                    while !head.ends_with(b"\r\n\r\n") {
                        head.push(stream.read_u8().await.unwrap());
                        assert!(head.len() < 8192, "unbounded fixture request");
                    }
                    assert!(head.starts_with(b"GET /effect HTTP/1.1\r\n"), "wrong wire target");
                    stream.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 5\r\n\r\nhello").await.unwrap();
                    stream.shutdown().await.unwrap();
                }
                written.send(()).unwrap();
            });
            let mut client = Client::new(origin, ConnectTimeout::new(Duration::from_secs(2)).unwrap()).unwrap();
            let request = client.request(http::Method::GET, ["effect"]).prepare().unwrap();
            let outcome = request.send(Instant::now() + Duration::from_secs(2), ResponseLimit::new(1024).unwrap()).await;
            assert!(matches!(&outcome, Exchange::NotDispatched(failure) if failure.failure() == ConnectFailure::Failed), "{outcome:?}");
            assert!(outcome.proves_unsent(), "untrusted TLS cannot dispatch HTTP");

            let mut roots = rustls::RootCertStore::empty();
            roots.add(cert.der().clone()).unwrap();
            let verifier = rustls::client::WebPkiServerVerifier::builder_with_provider(Arc::new(roots), provider).build().unwrap();
            let mut config = (*client.tls).clone();
            config.dangerous().set_certificate_verifier(verifier);
            client.tls = Arc::new(config);
            let request = client.request(http::Method::GET, ["effect"]).prepare().unwrap();
            let outcome = request.send(Instant::now() + Duration::from_secs(2), ResponseLimit::new(1024).unwrap()).await;
            assert_eq!(outcome.label(), "responded", "{outcome:?}");

            let request = client.request(http::Method::GET, ["effect"]).prepare().unwrap();
            let deadline = Instant::now() + Duration::from_millis(800);
            let exchange = request.send(deadline, ResponseLimit::new(1024).unwrap());
            tokio::pin!(exchange);
            tokio::select! {
                biased;
                ready = ready => ready.unwrap(),
                outcome = &mut exchange => panic!("exchange finished before the fixture wrote: {outcome:?}"),
            }
            tokio::time::sleep_until(deadline).await;
            let outcome = exchange.await;
            let Exchange::Responded(response) = outcome else { panic!("{outcome:?}") };
            assert_eq!(response.body().complete().unwrap(), "hello", "readable TLS body was discarded");
            peer.await.unwrap();
        }
    }).await.expect("bounded TLS regression");
    assert!(
        rustls::crypto::CryptoProvider::get_default().is_none(),
        "transport installed a process default"
    );
}
