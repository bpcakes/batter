//! Exercise the production TLS assembly with encrypted HTTP bytes blocked below
//! TLS, then trigger the read-side error flush after stopping the exchange.

use super::*;
use crate::{Exchange, ResponseLimit, send};
use bytes::Bytes;
use http_body_util::Full;
use hyper::client::conn::http1;
use hyper_util::rt::TokioIo;
use std::{
    future::{Future, poll_fn},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::io::{AsyncWriteExt, DuplexStream};

struct Wire {
    stream: DuplexStream,
    writable: Arc<AtomicBool>,
    written: Arc<AtomicUsize>,
    blocked: Arc<AtomicUsize>,
}

impl AsyncRead for Wire {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_read(cx, buf)
    }
}

impl AsyncWrite for Wire {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        if !self.writable.load(Ordering::SeqCst) {
            self.blocked.fetch_add(1, Ordering::SeqCst);
            return Poll::Pending;
        }
        let result = Pin::new(&mut self.stream).poll_write(cx, bytes);
        if let Poll::Ready(Ok(count)) = result {
            self.written.fetch_add(count, Ordering::SeqCst);
        }
        result
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_shutdown(cx)
    }
}

#[derive(Clone, Copy, Debug)]
enum Stop {
    ActiveControl,
    Deadline,
    Cancelled,
}

fn configs(
    version: &'static rustls::SupportedProtocolVersion,
) -> (Arc<rustls::ClientConfig>, Arc<rustls::ServerConfig>) {
    let rcgen::CertifiedKey { cert, signing_key } =
        rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut roots = rustls::RootCertStore::empty();
    roots.add(cert.der().clone()).unwrap();
    let mut client = rustls::ClientConfig::builder_with_provider(provider.clone())
        .with_protocol_versions(&[version])
        .unwrap()
        .with_root_certificates(roots)
        .with_no_client_auth();
    client.alpn_protocols = vec![b"http/1.1".to_vec()];
    let mut server = rustls::ServerConfig::builder_with_provider(provider)
        .with_protocol_versions(&[version])
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            vec![cert.der().clone()],
            rustls::pki_types::PrivatePkcs8KeyDer::from(signing_key.serialize_der()).into(),
        )
        .unwrap();
    server.alpn_protocols = vec![b"http/1.1".to_vec()];
    (Arc::new(client), Arc::new(server))
}

#[tokio::test(start_paused = true)]
async fn tls_error_reads_cannot_flush_buffered_http_after_deadline_or_cancellation() {
    for version in [&rustls::version::TLS12, &rustls::version::TLS13] {
        for stop in [Stop::ActiveControl, Stop::Deadline, Stop::Cancelled] {
            exercise_tls_error_read(version, stop).await;
        }
    }
}

async fn exercise_tls_error_read(version: &'static rustls::SupportedProtocolVersion, stop: Stop) {
    let (client, server) = configs(version);
    let cancel = AtomicBool::new(false);
    let gate = Gate {
        deadline: Instant::now() + Duration::from_secs(1),
        is_cancelled: || cancel.load(Ordering::SeqCst),
    };
    let writable = Arc::new(AtomicBool::new(true));
    let written = Arc::new(AtomicUsize::new(0));
    let blocked = Arc::new(AtomicUsize::new(0));
    let (stream, peer) = tokio::io::duplex(64 * 1024);
    let wire = Wire {
        stream,
        writable: writable.clone(),
        written: written.clone(),
        blocked: blocked.clone(),
    };
    let name = rustls::pki_types::ServerName::try_from("localhost").unwrap();
    let acceptor = tokio_rustls::TlsAcceptor::from(server);
    let (secured, peer) = tokio::join!(secure(wire, client, name, &gate), acceptor.accept(peer));
    let secured = secured.unwrap();
    let mut peer = peer.unwrap();
    assert!(
        !secured.inner.get_ref().1.wants_write(),
        "handshake left queued client records"
    );
    let before = written.load(Ordering::SeqCst);
    writable.store(false, Ordering::SeqCst);

    let connected = http1::handshake(TokioIo::new(secured)).await.unwrap();
    let body_len = 8192;
    let request = http::Request::builder()
        .method("POST")
        .uri("/effects")
        .header("host", "localhost")
        .body(Full::new(Bytes::from(vec![b'x'; body_len])))
        .unwrap();
    let timer = tokio::time::sleep_until(gate.deadline);
    let cancelled = poll_fn(|_| {
        if cancel.load(Ordering::SeqCst) {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    });
    tokio::pin!(timer, cancelled);
    let exchange = send::send_on(
        connected,
        request,
        &gate,
        ResponseLimit::new(1024).unwrap(),
        timer,
        cancelled,
    );
    tokio::pin!(exchange);
    poll_fn(|cx| {
        assert!(
            exchange.as_mut().poll(cx).is_pending(),
            "exchange finished before its writer stalled"
        );
        Poll::Ready(())
    })
    .await;
    assert!(
        blocked.load(Ordering::SeqCst) > 0,
        "no encrypted request reached the blocked raw writer"
    );
    assert_eq!(
        written.load(Ordering::SeqCst),
        before,
        "blocked writer leaked request bytes"
    );
    match stop {
        Stop::ActiveControl => {}
        Stop::Deadline => tokio::time::advance(Duration::from_secs(1)).await,
        Stop::Cancelled => cancel.store(true, Ordering::SeqCst),
    }
    writable.store(true, Ordering::SeqCst);
    // A syntactically framed but unauthenticatable application-data
    // record makes process_new_packets fail. tokio-rustls then calls
    // write_io from its read path to send an alert, draining any older
    // queued HTTP ciphertext first unless the TCP guard prevents it.
    peer.get_mut()
        .0
        .write_all(&[0x17, 0x03, 0x03, 0, 1, 0])
        .await
        .unwrap();
    let outcome = exchange.await;
    assert!(
        matches!(outcome, Exchange::Unknown(_)),
        "{stop:?}: {outcome:?}"
    );
    let after = written.load(Ordering::SeqCst);
    if matches!(stop, Stop::ActiveControl) {
        assert!(
            after - before > body_len,
            "control did not flush the buffered HTTP request before its alert"
        );
    } else {
        assert_eq!(
            after, before,
            "{stop:?}: TLS read flushed request ciphertext after stopping"
        );
    }
}
