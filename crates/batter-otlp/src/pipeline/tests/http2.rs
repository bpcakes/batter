//! Exercise the native protocol retry classifier, including its default control.

use super::{AttemptSlot, BoundedClient, client_builder};
use crate::diagnostics::ExportFailure;
use bytes::Bytes;
use opentelemetry_http::HttpClient;
use std::{sync::Arc, time::Duration};
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
    let slot = Arc::new(AttemptSlot::default());
    let client = BoundedClient {
        // Use h2c to isolate protocol handling from certificates/ALPN. All
        // transport policy comes from the same builder used by production.
        client: builder.http2_prior_knowledge().build().unwrap(),
        slot: slot.clone(),
    };
    slot.begin(Instant::now() + Duration::from_secs(2));
    let request = http::Request::post(endpoint)
        .body(Bytes::from_static(b"metrics"))
        .unwrap();
    assert!(client.send_bytes(request).await.is_err());
    assert_eq!(slot.finish().outcome, Some(Err(ExportFailure::Transport)));
    stop.send(()).unwrap();
    server.await.unwrap()
}

#[tokio::test]
async fn consumer_http2_feature_cannot_enable_protocol_retries() {
    tokio::time::timeout(Duration::from_secs(10), async {
        // Positive control: the pinned native default really retries this NACK.
        assert_eq!(
            refused_streams(reqwest::Client::builder().no_proxy()).await,
            3
        );
        assert_eq!(refused_streams(client_builder()).await, 1);
    })
    .await
    .expect("bounded HTTP/2 protocol regression");
}
