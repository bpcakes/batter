//! Bounded OTLP/HTTP transport retaining typed outcomes beside the SDK result.
//!
//! The upstream exporter formats custom client errors and non-success response
//! bodies into strings. This client therefore records its own closed outcome
//! for the single in-flight attempt, returns fixed error values and never hands
//! a collector body back upstream. The export owner reads the typed outcome;
//! it never inspects upstream error text.

#[path = "attempt.rs"]
mod attempt;
#[path = "exporter.rs"]
mod exporter;
#[path = "tls.rs"]
mod tls;
pub(crate) use exporter::Exporter;

use crate::diagnostics::ExportFailure;
use async_trait::async_trait;
use bytes::Bytes;
use opentelemetry_http::{HttpClient, HttpError};
use opentelemetry_proto::tonic::collector::metrics::v1::ExportMetricsServiceResponse;
use prost::Message;
use reqwest::{Response, redirect::Policy};
use std::{fmt, sync::Arc};
use tokio::time::timeout_at;

/// Largest encoded request sent to the collector. Larger requests are refused
/// before dispatch. The complete foundation catalog with maximum-length names
/// encodes to less than half of this ceiling.
pub(crate) const PAYLOAD_MAX_BYTES: usize = 2 * 1024 * 1024;
/// Largest collector response body read.
pub(crate) const RESPONSE_MAX_BYTES: usize = 64 * 1024;
const PROTOBUF: &str = "application/x-protobuf";

/// Chosen by preparation, never inferred from a response or hostname.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ResponseProfile {
    Otlp,
    VictoriaMetrics,
}

/// Constructed only after bounded body reading and backend response validation.
struct Acknowledged;

/// Fixed error returned upstream; its formatting contains no collector data.
#[derive(Debug, thiserror::Error)]
#[error("bounded metrics transport did not acknowledge the request")]
struct Unacknowledged;

/// The OpenTelemetry HTTP client used by the protected OTLP adapter.
struct BoundedClient {
    client: reqwest::Client,
    gate: Arc<attempt::Gate>,
    profile: ResponseProfile,
}

impl fmt::Debug for BoundedClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("BoundedClient")
    }
}

impl BoundedClient {
    /// Build an inert client: no redirects, no proxy discovery, no connection.
    fn new(gate: Arc<attempt::Gate>, profile: ResponseProfile) -> Result<Self, reqwest::Error> {
        let client = client_builder().build()?;
        Ok(Self {
            client,
            gate,
            profile,
        })
    }

    async fn exchange(
        &self,
        request: http::Request<Bytes>,
        preparing: attempt::Preparing,
    ) -> Result<(), Unacknowledged> {
        if request.body().len() > PAYLOAD_MAX_BYTES {
            preparing.fail(ExportFailure::PayloadTooLarge);
            return Err(Unacknowledged);
        }
        let request = match reqwest::Request::try_from(request) {
            Ok(request) => request,
            Err(_) => {
                preparing.fail(ExportFailure::Encoding);
                return Err(Unacknowledged);
            }
        };
        let sending = preparing.send().ok_or(Unacknowledged)?;
        let deadline = sending.deadline();
        let response = match timeout_at(deadline, self.client.execute(request)).await {
            Ok(Ok(response)) => response,
            result => {
                let failure = match result {
                    Err(_) => ExportFailure::HeadersTimedOut,
                    Ok(Err(error)) if error.is_connect() => ExportFailure::Refused,
                    _ => ExportFailure::Transport,
                };
                sending.fail(failure);
                return Err(Unacknowledged);
            }
        };
        let status = response.status();
        if status != reqwest::StatusCode::OK {
            // The status is the whole retained evidence; the body is not read.
            sending.fail(ExportFailure::Status(status.as_u16()));
            return Err(Unacknowledged);
        }
        let receiving = sending.receive().ok_or(Unacknowledged)?;
        let validated = match timeout_at(deadline, self.validate(response)).await {
            Err(_) => Err(ExportFailure::BodyTimedOut),
            Ok(result) => result,
        };
        if receiving.complete(validated) {
            Ok(())
        } else {
            Err(Unacknowledged)
        }
    }

    async fn validate(&self, response: Response) -> Result<Acknowledged, ExportFailure> {
        let content_type = response.headers().get(http::header::CONTENT_TYPE);
        let missing_content_type = content_type.is_none();
        let protobuf = content_type
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<mime::Mime>().ok())
            .is_some_and(|value| value.essence_str() == PROTOBUF);
        let body = bounded_body(response).await?;
        // Only this explicit profile accepts the backend's empty untyped 200.
        if self.profile == ResponseProfile::VictoriaMetrics
            && missing_content_type
            && body.is_empty()
        {
            return Ok(Acknowledged);
        }
        if !protobuf {
            return Err(ExportFailure::MalformedResponse);
        }
        classify_response(&body)?;
        Ok(Acknowledged)
    }
}

fn client_builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .tls_backend_preconfigured(tls::config())
        .http1_only()
        .no_gzip()
        .no_brotli()
        .no_deflate()
        .no_zstd()
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
        let preparing = self.gate.claim().ok_or(Unacknowledged)?;
        self.exchange(request, preparing).await?;
        // A fixed empty success response: upstream never sees the collector body.
        Ok(http::Response::new(Bytes::new()))
    }
}

#[cfg(test)]
#[path = "tests/attempt_transport.rs"]
mod attempt_transport;

#[cfg(test)]
#[path = "tests/http2.rs"]
mod http2;

#[cfg(test)]
#[path = "tests/tls.rs"]
mod tls_tests;
