//! Exercise the native protocol retry classifier, including its default control.

use super::{BoundedClient, attempt::Owner, client_builder};
use crate::diagnostics::{ExportFailure, ExportOutcome};
use bytes::Bytes;
use opentelemetry_http::HttpClient;
use std::time::Duration;
use tokio::{net::TcpListener, sync::oneshot, time::Instant};

async fn refused_streams(builder: reqwest::ClientBuilder) -> usize {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/v1/metrics", listener.local_addr().unwrap());
    let (stop, mut stopped) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut connection = h2::server::handshake(stream).await.unwrap();
        let mut requests = 0;
        loop {
            tokio::select! {
                _ = &mut stopped => return requests,
                request = connection.accept() => {
                    let (request, mut response) = request.unwrap().unwrap();
                    assert_eq!(request.method(), http::Method::POST);
                    assert_eq!(request.uri().path(), "/v1/metrics");
                    requests += 1;
                    response.send_reset(h2::Reason::REFUSED_STREAM);
                }
            }
        }
    });
    let mut owner = Owner::default();
    let client = BoundedClient {
        // Use h2c to isolate protocol handling from certificates/ALPN. All
        // transport policy comes from the same builder used by production.
        client: builder.http2_prior_knowledge().build().unwrap(),
        gate: owner.gate(),
        profile: super::ResponseProfile::Otlp,
    };
    let request = http::Request::post(endpoint)
        .body(Bytes::from_static(b"metrics"))
        .unwrap();
    let outcome = owner
        .run(Instant::now() + Duration::from_secs(2), || {
            client.send_bytes(request)
        })
        .await;
    assert_eq!(outcome, ExportOutcome::Failed(ExportFailure::Transport));
    stop.send(()).unwrap();
    server.await.unwrap()
}

#[tokio::test]
async fn consumer_http2_feature_cannot_enable_protocol_retries() {
    tokio::time::timeout(Duration::from_secs(10), async {
        // Positive control: the pinned native default really retries this NACK.
        assert_eq!(
            refused_streams(
                reqwest::Client::builder()
                    .tls_backend_preconfigured(super::tls::config())
                    .no_proxy()
            )
            .await,
            3
        );
        assert_eq!(refused_streams(client_builder()).await, 1);
    })
    .await
    .expect("bounded HTTP/2 protocol regression");
}
