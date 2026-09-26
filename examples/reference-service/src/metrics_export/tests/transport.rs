//! Typed collector outcomes, transport bounds and noninterference.

use super::collector::{BODY_MARKER, Behavior};
use super::{
    super::{
        Schedule, classify,
        transport::{AttemptSlot, BoundedClient, Evidence, PAYLOAD_MAX_BYTES, Phase},
    },
    Harness, key, pipeline,
};
use crate::diagnostics::{ExportFailure, ExportOutcome};
use batter::telemetry::metrics::{self as catalog, facade::Recorder};
use bytes::Bytes;
use opentelemetry_http::HttpClient;
use std::{sync::Arc, time::Duration};
use tokio::time::Instant;

const SHORT: Schedule = Schedule {
    interval: Duration::from_secs(3600),
    attempt: Duration::from_millis(300),
    final_allowance: Duration::from_millis(300),
};

fn record_one(recorder: &dyn Recorder) {
    let labels = [("outcome", "succeeded")];
    recorder
        .register_counter(&key(catalog::CLEANUP_HOOKS, &labels), &super::METADATA)
        .increment(1);
}

#[test]
fn every_collector_response_maps_to_one_closed_outcome_without_retry() {
    use ExportFailure::*;
    for (behavior, expected) in [
        (Behavior::Accept, ExportOutcome::Acknowledged),
        (
            Behavior::PartialReject(3),
            ExportOutcome::Failed(PartiallyRejected { rejected_points: 3 }),
        ),
        (
            Behavior::PartialReject(-1),
            ExportOutcome::Failed(MalformedResponse),
        ),
        (Behavior::Status(503), ExportOutcome::Failed(Status(503))),
        (Behavior::Status(400), ExportOutcome::Failed(Status(400))),
        (
            Behavior::Malformed,
            ExportOutcome::Failed(MalformedResponse),
        ),
        (
            Behavior::WrongContentType,
            ExportOutcome::Failed(MalformedResponse),
        ),
        (
            Behavior::DeclaredOversized,
            ExportOutcome::Failed(ResponseTooLarge),
        ),
        (
            Behavior::StreamedOversized,
            ExportOutcome::Failed(ResponseTooLarge),
        ),
        (
            Behavior::StallHeaders,
            ExportOutcome::Failed(HeadersTimedOut),
        ),
        (Behavior::StallBody, ExportOutcome::Failed(BodyTimedOut)),
    ] {
        let harness = Harness::new(behavior);
        let (recorder, mut session) = harness.pipeline(SHORT);
        let started = Instant::now();
        let outcome = harness.run(&recorder, async {
            record_one(&recorder);
            session.attempt(Instant::now() + SHORT.attempt).await
        });
        assert_eq!(outcome, expected, "{behavior:?}");
        assert!(started.elapsed() < SHORT.attempt + Duration::from_secs(1));
        // One dispatch per attempt: partial success and failures are never retried.
        assert_eq!(harness.collector().requests().len(), 1, "{behavior:?}");
        assert!(!format!("{outcome:?}").contains(BODY_MARKER));
    }
}

#[test]
fn refused_connection_is_a_known_non_dispatch() {
    let harness = Harness::new(Behavior::Accept);
    let endpoint = harness.runtime.block_on(async {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        format!("http://{address}/v1/metrics")
    });
    let (recorder, mut session) = pipeline(&endpoint, SHORT);
    let outcome = harness.run(&recorder, async {
        record_one(&recorder);
        session.attempt(Instant::now() + SHORT.attempt).await
    });
    assert_eq!(outcome, ExportOutcome::Failed(ExportFailure::Refused));
}

#[test]
fn oversized_payloads_and_requests_outside_an_attempt_are_never_sent() {
    let harness = Harness::new(Behavior::Accept);
    let endpoint = harness.collector().endpoint();
    let slot = Arc::new(AttemptSlot::default());
    let client = BoundedClient::new(slot.clone()).unwrap();
    let request = |body: Vec<u8>| {
        http::Request::post(endpoint.as_str())
            .header(http::header::CONTENT_TYPE, "application/x-protobuf")
            .body(Bytes::from(body))
            .unwrap()
    };
    harness.runtime.block_on(async {
        // No open attempt: only the export owner may authorize a send.
        assert!(client.send_bytes(request(Vec::new())).await.is_err());
        slot.begin(Instant::now() + Duration::from_secs(1));
        let oversized = request(vec![0; PAYLOAD_MAX_BYTES + 1]);
        let error = client.send_bytes(oversized).await.unwrap_err();
        assert!(!format!("{error:?} {error}").contains(BODY_MARKER));
        assert_eq!(
            slot.finish(),
            Evidence {
                phase: Phase::Prepared,
                outcome: Some(Err(ExportFailure::PayloadTooLarge)),
            }
        );
    });
    assert!(harness.collector().requests().is_empty());
}

#[test]
fn outer_deadline_classification_uses_only_typed_evidence() {
    let evidence = |phase, outcome| Evidence { phase, outcome };
    let failed = ExportOutcome::Failed;
    assert_eq!(
        classify(true, evidence(Phase::Receiving, None)),
        failed(ExportFailure::BodyTimedOut)
    );
    for phase in [Phase::Prepared, Phase::Sending] {
        assert_eq!(
            classify(true, evidence(phase, None)),
            failed(ExportFailure::HeadersTimedOut)
        );
    }
    assert_eq!(
        classify(false, evidence(Phase::Prepared, None)),
        failed(ExportFailure::Encoding)
    );
    // Typed transport evidence wins over the outer race.
    assert_eq!(
        classify(true, evidence(Phase::Receiving, Some(Ok(())))),
        ExportOutcome::Acknowledged
    );
    assert_eq!(
        classify(
            false,
            evidence(Phase::Sending, Some(Err(ExportFailure::Refused)))
        ),
        failed(ExportFailure::Refused)
    );
}

#[test]
fn stalled_collector_and_series_storm_change_no_result_or_bound() {
    let harness = Harness::new(Behavior::StallHeaders);
    let (recorder, mut session) = harness.pipeline(SHORT);
    let state = session.resources.state.clone();
    let context = batter::operation::OperationOwner::new(Duration::from_secs(5))
        .unwrap()
        .into_context();
    let (outcome, value) = harness.run(&recorder, async {
        tokio::join!(session.attempt(Instant::now() + SHORT.attempt), async {
            // The only request is stalled at the collector. Recording neither
            // waits for it nor queues anything behind it.
            let started = Instant::now();
            for index in 0..10_000 {
                let storm = batter::telemetry::metrics::facade::Key::from_name(format!(
                    "library_series_{index}"
                ));
                recorder
                    .register_counter(&storm, &super::METADATA)
                    .increment(1);
            }
            let value = context
                .run("storm.operation", |_| async { Ok::<_, std::io::Error>(7) })
                .await;
            assert!(started.elapsed() < SHORT.attempt);
            value
        })
    });
    assert_eq!(value.unwrap(), 7);
    assert_eq!(
        outcome,
        ExportOutcome::Failed(ExportFailure::HeadersTimedOut)
    );
    assert_eq!(state.rejections().unknown_names, 10_000);
    // Only the operation's completion counter and duration histogram exist.
    assert_eq!(state.admitted(), 2);
    harness.collector().set(Behavior::Accept);
    let (_, report) = harness.run(&recorder, async {
        session
            .around(async {}, |_| crate::diagnostics::FinalCoverage::Reported)
            .await
    });
    assert_eq!(report.final_export, ExportOutcome::Acknowledged);
    let requests = harness.collector().requests();
    assert_eq!(requests.len(), 2);
    let exported = super::series(&requests[1]);
    let labels = [("operation", "storm.operation"), ("outcome", "succeeded")];
    assert_eq!(
        super::counter(&exported, catalog::OPERATION_COMPLETIONS, &labels),
        Some(1)
    );
    assert_eq!(exported.len(), 2);
}
