//! Transport regressions shared with the reference provider's private client.

use std::{sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

pub async fn assert_bundled_tls(client: reqwest::Client) {
    assert!(rustls::crypto::CryptoProvider::get_default().is_none());
    tokio::time::timeout(Duration::from_secs(5), async {
        let rcgen::CertifiedKey { cert, signing_key } =
            rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        for version in [&rustls::version::TLS12, &rustls::version::TLS13] {
            let provider = Arc::new(rustls::crypto::ring::default_provider());
            let mut server = rustls::ServerConfig::builder_with_provider(provider.clone())
                .with_protocol_versions(&[version]).unwrap()
                .with_no_client_auth()
                .with_single_cert(vec![cert.der().clone()],
                    rustls::pki_types::PrivatePkcs8KeyDer::from(signing_key.serialize_der()).into())
                .unwrap();
            server.alpn_protocols = vec![b"http/1.1".to_vec()];
            let server = Arc::new(server);
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let endpoint = format!("https://localhost:{}/", listener.local_addr().unwrap().port());
            let peer = tokio::spawn(async move {
                for trusted in [false, true] {
                    let (stream, _) = listener.accept().await.unwrap();
                    let hello = tokio_rustls::LazyConfigAcceptor::new(
                        rustls::server::Acceptor::default(), stream).await.unwrap();
                    assert_eq!(hello.client_hello().alpn().unwrap().collect::<Vec<_>>(),
                        vec![b"http/1.1".as_slice()]);
                    let stream = hello.into_stream(server.clone()).await;
                    if trusted {
                        let mut stream = stream.unwrap();
                        let mut request = [0; 4096];
                        assert!(stream.read(&mut request).await.unwrap() > 0);
                        stream.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\nconnection: close\r\n\r\n").await.unwrap();
                        stream.shutdown().await.unwrap();
                    } else {
                        assert!(stream.is_err(), "untrusted fixture certificate was accepted");
                    }
                }
            });
            assert!(client.get(&endpoint).send().await.unwrap_err().is_connect());

            // Positive control: the same fixture succeeds only with explicit trust.
            let mut roots = rustls::RootCertStore::empty();
            roots.add(cert.der().clone()).unwrap();
            let mut config = rustls::ClientConfig::builder_with_provider(provider)
                .with_protocol_versions(&[version]).unwrap()
                .with_root_certificates(roots).with_no_client_auth();
            config.alpn_protocols = vec![b"http/1.1".to_vec()];
            let trusted = reqwest::Client::builder().tls_backend_preconfigured(config)
                .no_proxy().build().unwrap();
            assert_eq!(trusted.get(&endpoint).send().await.unwrap().status(), 200);
            peer.await.unwrap();
        }
    }).await.expect("bounded TLS regression");
    assert!(rustls::crypto::CryptoProvider::get_default().is_none());
}

pub async fn assert_no_decompression(client: reqwest::Client, control: reqwest::Client) {
    const ENCODED: &[u8] = &[
        31, 139, 8, 0, 0, 0, 0, 0, 0, 255, 75, 203, 172, 40, 41, 45, 74, 85, 72, 202, 79, 169, 4,
        0, 58, 203, 161, 35, 12, 0, 0, 0,
    ];
    tokio::time::timeout(Duration::from_secs(5), async {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/", listener.local_addr().unwrap());
        let peer = tokio::spawn(async move {
            for compressed in [false, true] {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut head = Vec::new();
                while !head.ends_with(b"\r\n\r\n") {
                    head.push(stream.read_u8().await.unwrap());
                    assert!(head.len() < 8192);
                }
                let head = String::from_utf8(head).unwrap().to_ascii_lowercase();
                assert_eq!(head.contains("accept-encoding:"), compressed);
                let reply = format!("HTTP/1.1 200 OK\r\ncontent-encoding: gzip\r\ncontent-length: {}\r\nconnection: close\r\n\r\n", ENCODED.len());
                stream.write_all(reply.as_bytes()).await.unwrap();
                stream.write_all(ENCODED).await.unwrap();
            }
        });
        assert_eq!(client.get(&endpoint).send().await.unwrap().bytes().await.unwrap().as_ref(), ENCODED);
        assert_eq!(control.get(&endpoint).send().await.unwrap().bytes().await.unwrap().as_ref(), b"fixture body");
        peer.await.unwrap();
    }).await.expect("bounded compression regression");
}
