//! Actual HTTP dispatch and cancellation at the owned SDK-client boundary.

use super::{BoundedClient, PAYLOAD_MAX_BYTES, ResponseProfile, attempt::Owner};
use crate::diagnostics::{ExportFailure, ExportOutcome};
use crate::tests::collector::{self, Behavior, Collector};
use bytes::Bytes;
use opentelemetry_http::HttpClient;
use std::{sync::Arc, time::Duration};
use tokio::time::Instant;

fn request(endpoint: &str, body: Vec<u8>) -> http::Request<Bytes> {
    http::Request::post(endpoint)
        .header(http::header::CONTENT_TYPE, "application/x-protobuf")
        .body(Bytes::from(body))
        .unwrap()
}

#[tokio::test]
async fn unarmed_oversized_and_completed_attempts_cannot_dispatch() {
    let collector = Collector::start(Behavior::Accept).await.unwrap();
    let endpoint = collector.endpoint();
    let mut owner = Owner::default();
    let client = BoundedClient::new(owner.gate(), ResponseProfile::Otlp).unwrap();
    assert!(client.send_bytes(request(&endpoint, vec![])).await.is_err());
    let outcome = owner
        .run(Instant::now() + Duration::from_secs(2), || async {
            let error = client
                .send_bytes(request(&endpoint, vec![0; PAYLOAD_MAX_BYTES + 1]))
                .await
                .unwrap_err();
            assert!(!format!("{error:?} {error}").contains(collector::BODY_MARKER));
            assert!(client.send_bytes(request(&endpoint, vec![])).await.is_err());
        })
        .await;
    assert_eq!(
        outcome,
        ExportOutcome::Failed(ExportFailure::PayloadTooLarge)
    );
    assert!(collector.requests().is_empty());
    assert!(client.send_bytes(request(&endpoint, vec![])).await.is_err());
    collector.close().await.unwrap();
}

#[tokio::test]
async fn concurrent_and_repeated_callbacks_dispatch_exactly_once() {
    let collector = Collector::start(Behavior::Accept).await.unwrap();
    let endpoint = collector.endpoint();
    let mut owner = Owner::default();
    let client = BoundedClient::new(owner.gate(), ResponseProfile::Otlp).unwrap();
    let outcome = owner
        .run(Instant::now() + Duration::from_secs(2), || async {
            let (first, second) = tokio::join!(
                client.send_bytes(request(&endpoint, vec![])),
                client.send_bytes(request(&endpoint, vec![])),
            );
            assert!(first.is_ok());
            assert!(second.is_err());
            assert!(client.send_bytes(request(&endpoint, vec![])).await.is_err());
        })
        .await;
    assert_eq!(outcome, ExportOutcome::Acknowledged);
    assert_eq!(collector.requests().len(), 1);
    assert!(client.send_bytes(request(&endpoint, vec![])).await.is_err());
    collector.close().await.unwrap();
}

#[tokio::test]
async fn dropping_stalled_exports_allows_a_fresh_attempt_without_replaying_the_old_one() {
    for behavior in [Behavior::StallHeaders, Behavior::StallBody] {
        let collector = Collector::start(behavior).await.unwrap();
        let endpoint = collector.endpoint();
        let mut owner = Owner::default();
        let mut client = BoundedClient::new(owner.gate(), ResponseProfile::Otlp).unwrap();
        let body_pending = Arc::new(tokio::sync::Notify::new());
        client.body_pending = Some(body_pending.clone());
        let mut pending = Box::pin(owner.run(Instant::now() + Duration::from_secs(3), || {
            client.send_bytes(request(&endpoint, vec![]))
        }));
        let checkpoint = async {
            match behavior {
                Behavior::StallHeaders => {
                    collector
                        .wait_requests(1, Duration::from_secs(2))
                        .await
                        .unwrap();
                }
                Behavior::StallBody => {
                    // Request receipt precedes headers. Wait for the actual body
                    // future to return Pending before exercising its cancellation.
                    tokio::time::timeout(Duration::from_secs(2), body_pending.notified())
                        .await
                        .unwrap();
                }
                _ => unreachable!(),
            }
        };
        tokio::select! {
            outcome = &mut pending => panic!("stalled attempt returned: {outcome:?}"),
            () = checkpoint => {}
        }
        drop(pending);
        assert!(client.send_bytes(request(&endpoint, vec![])).await.is_err());
        collector.set(Behavior::Accept);
        let outcome = owner
            .run(Instant::now() + Duration::from_secs(2), || {
                client.send_bytes(request(&endpoint, vec![]))
            })
            .await;
        assert_eq!(outcome, ExportOutcome::Acknowledged);
        assert_eq!(collector.requests().len(), 2);
        collector.close().await.unwrap();
    }
}
