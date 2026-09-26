//! Bounded OTLP/HTTP transport retaining typed outcomes beside the SDK result.
//!
//! The upstream exporter formats custom client errors and non-success response
//! bodies into strings. This client therefore records its own closed outcome
//! for the single in-flight attempt, returns fixed error values and never hands
//! a collector body back upstream. The export owner reads the typed outcome;
//! it never inspects upstream error text.

use crate::diagnostics::ExportFailure;
use async_trait::async_trait;
use bytes::Bytes;
use opentelemetry_http::{HttpClient, HttpError};
use opentelemetry_proto::tonic::collector::metrics::v1::ExportMetricsServiceResponse;
use prost::Message;
use reqwest::{Response, redirect::Policy};
use std::{
    fmt,
    sync::{Arc, Mutex, PoisonError},
};
use tokio::time::{Instant, timeout_at};

/// Largest encoded request sent to the collector. Larger requests are refused
/// before dispatch. The complete foundation catalog with maximum-length names
/// encodes to less than half of this ceiling.
pub(crate) const PAYLOAD_MAX_BYTES: usize = 2 * 1024 * 1024;
/// Largest collector response body read.
pub(crate) const RESPONSE_MAX_BYTES: usize = 64 * 1024;
const PROTOBUF: &str = "application/x-protobuf";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Phase {
    /// The owner selected the deadline; the transport has not been invoked.
    Prepared,
    /// Connecting, writing the request or waiting for response headers.
    Sending,
    /// Reading the response body.
    Receiving,
}

struct Attempt {
    deadline: Instant,
    phase: Phase,
    outcome: Option<Result<(), ExportFailure>>,
}

/// Typed evidence for the single in-flight attempt. Exports are serial, so one
/// slot is the whole transport history.
#[derive(Default)]
pub(crate) struct AttemptSlot(Mutex<Option<Attempt>>);

/// What the transport observed for one attempt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Evidence {
    pub(crate) phase: Phase,
    pub(crate) outcome: Option<Result<(), ExportFailure>>,
}

impl AttemptSlot {
    fn lock(&self) -> std::sync::MutexGuard<'_, Option<Attempt>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn begin(&self, deadline: Instant) {
        *self.lock() = Some(Attempt {
            deadline,
            phase: Phase::Prepared,
            outcome: None,
        });
    }

    pub(crate) fn finish(&self) -> Evidence {
        match self.lock().take() {
            Some(attempt) => Evidence {
                phase: attempt.phase,
                outcome: attempt.outcome,
            },
            None => Evidence {
                phase: Phase::Prepared,
                outcome: None,
            },
        }
    }

    fn deadline(&self) -> Option<Instant> {
        self.lock()
            .as_ref()
            .filter(|attempt| attempt.outcome.is_none())
            .map(|attempt| attempt.deadline)
    }

    fn enter(&self, phase: Phase) {
        if let Some(attempt) = self.lock().as_mut() {
            attempt.phase = phase;
        }
    }

    fn record(&self, outcome: Result<(), ExportFailure>) {
        if let Some(attempt) = self.lock().as_mut() {
            attempt.outcome = Some(outcome);
        }
    }
}

/// Fixed error returned upstream; its formatting contains no collector data.
#[derive(Debug, thiserror::Error)]
#[error("bounded metrics transport did not acknowledge the request")]
struct Unacknowledged;

/// The OpenTelemetry HTTP client used by the protected OTLP adapter.
pub(crate) struct BoundedClient {
    client: reqwest::Client,
    slot: Arc<AttemptSlot>,
}

impl fmt::Debug for BoundedClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("BoundedClient")
    }
}

impl BoundedClient {
    /// Build an inert client: no redirects, no proxy discovery, no connection.
    pub(crate) fn new(slot: Arc<AttemptSlot>) -> Result<Self, reqwest::Error> {
        let client = client_builder().build()?;
        Ok(Self { client, slot })
    }

    async fn exchange(
        &self,
        request: http::Request<Bytes>,
        deadline: Instant,
    ) -> Result<(), ExportFailure> {
        if request.body().len() > PAYLOAD_MAX_BYTES {
            return Err(ExportFailure::PayloadTooLarge);
        }
        let request = reqwest::Request::try_from(request).map_err(|_| ExportFailure::Encoding)?;
        self.slot.enter(Phase::Sending);
        let response = match timeout_at(deadline, self.client.execute(request)).await {
            Err(_) => return Err(ExportFailure::HeadersTimedOut),
            Ok(Err(error)) if error.is_connect() => return Err(ExportFailure::Refused),
            Ok(Err(_)) => return Err(ExportFailure::Transport),
            Ok(Ok(response)) => response,
        };
        let status = response.status();
        if !status.is_success() {
            // The status is the whole retained evidence; the body is not read.
            return Err(ExportFailure::Status(status.as_u16()));
        }
        let protobuf = response
            .headers()
            .get(http::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.starts_with(PROTOBUF));
        self.slot.enter(Phase::Receiving);
        let body = match timeout_at(deadline, bounded_body(response)).await {
            Err(_) => return Err(ExportFailure::BodyTimedOut),
            Ok(body) => body?,
        };
        if !body.is_empty() && !protobuf {
            return Err(ExportFailure::MalformedResponse);
        }
        classify_response(&body)
    }
}

fn client_builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .redirect(Policy::none())
        .no_proxy()
        // Consumer feature unification can enable HTTP/2 protocol retries.
        .retry(reqwest::retry::never())
}

/// Decode the OTLP response. Only a decoded response without rejected points is
/// acknowledgement; any HTTP 2xx alone is not.
pub(crate) fn classify_response(body: &[u8]) -> Result<(), ExportFailure> {
    let response =
        ExportMetricsServiceResponse::decode(body).map_err(|_| ExportFailure::MalformedResponse)?;
    match response.partial_success {
        Some(partial) if partial.rejected_data_points > 0 => {
            Err(ExportFailure::PartiallyRejected {
                rejected_points: partial.rejected_data_points.unsigned_abs(),
            })
        }
        Some(partial) if partial.rejected_data_points < 0 => Err(ExportFailure::MalformedResponse),
        // A warning message without rejected points is accepted; its text is
        // collector-controlled and is not retained.
        _ => Ok(()),
    }
}

async fn bounded_body(mut response: Response) -> Result<Vec<u8>, ExportFailure> {
    if response
        .content_length()
        .is_some_and(|length| length > RESPONSE_MAX_BYTES as u64)
    {
        return Err(ExportFailure::ResponseTooLarge);
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| ExportFailure::Transport)?
    {
        if body.len().saturating_add(chunk.len()) > RESPONSE_MAX_BYTES {
            return Err(ExportFailure::ResponseTooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

#[async_trait]
impl HttpClient for BoundedClient {
    async fn send_bytes(
        &self,
        request: http::Request<Bytes>,
    ) -> Result<http::Response<Bytes>, HttpError> {
        let Some(deadline) = self.slot.deadline() else {
            // Only the export owner opens an attempt; nothing else may send.
            return Err(Box::new(Unacknowledged));
        };
        let outcome = self.exchange(request, deadline).await;
        self.slot.record(outcome);
        match outcome {
            // A fixed empty success response: upstream never sees the body.
            Ok(()) => Ok(http::Response::new(Bytes::new())),
            Err(_) => Err(Box::new(Unacknowledged)),
        }
    }
}

#[cfg(test)]
#[path = "tests/http2.rs"]
mod http2;
