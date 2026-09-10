//! Response facts and tracing lifetime, independent of admission policy.

#[cfg(test)]
mod tests;

use super::{CorrelationId, HttpObservationLevel};
use axum::{
    extract::{MatchedPath, Request},
    http::{Method, StatusCode},
    response::Response,
};
use std::future::Future;
use tokio::time::Instant;
use tracing::{Instrument, Level};

pub(super) async fn observe_response<F: Future<Output = Response>>(
    request: Request,
    run: impl FnOnce(Request) -> F,
) -> Response {
    let mut observation = HttpObservation::new(&request);
    let span = observation.context.clone();
    let response = run(request).instrument(span).await;
    observation.status = Some(response.status());
    observation.level = response.extensions().get::<HttpObservationLevel>().copied();
    response
}

struct HttpObservation {
    span: tracing::Span,
    context: tracing::Span,
    correlation: Option<CorrelationId>,
    method: &'static str,
    route: Option<MatchedPath>,
    started: Instant,
    status: Option<StatusCode>,
    level: Option<HttpObservationLevel>,
}

impl HttpObservation {
    fn new(request: &Request) -> Self {
        let method = match *request.method() {
            Method::GET => "GET",
            Method::HEAD => "HEAD",
            Method::POST => "POST",
            Method::PUT => "PUT",
            Method::DELETE => "DELETE",
            Method::CONNECT => "CONNECT",
            Method::OPTIONS => "OPTIONS",
            Method::TRACE => "TRACE",
            Method::PATCH => "PATCH",
            _ => "OTHER",
        };
        let route = request.extensions().get::<MatchedPath>().cloned();
        let span = tracing::info_span!(
            target: "batter",
            "batter.http",
            method,
            route = route.as_ref().map(MatchedPath::as_str).unwrap_or("<unmatched>"),
            status = tracing::field::Empty,
            http_outcome = tracing::field::Empty,
            latency_ms = tracing::field::Empty,
        );
        // Choose once on first poll. A disabled explicit parent means a root
        // event, and looking up a fallback at Drop could adopt another request.
        let context = span.clone().or_current();
        Self {
            span,
            context,
            correlation: request.extensions().get::<CorrelationId>().cloned(),
            method,
            route,
            started: Instant::now(),
            status: None,
            level: None,
        }
    }
}

impl Drop for HttpObservation {
    fn drop(&mut self) {
        let outcome = match self.status {
            Some(status) if status.is_server_error() => "server_error",
            Some(status) if status.is_client_error() => "client_error",
            Some(_) => "completed",
            None => "dropped",
        };
        let latency_ms = self.started.elapsed().as_secs_f64() * 1_000.0;
        if let Some(status) = self.status {
            self.span.record("status", status.as_u16());
        }
        self.span.record("http_outcome", outcome);
        self.span.record("latency_ms", latency_ms);
        let level = self.level.map(|level| level.0).unwrap_or_else(|| {
            if self.status.is_none_or(|status| status.is_server_error()) {
                Level::WARN
            } else {
                Level::INFO
            }
        });
        self.emit_completion(level, outcome, latency_ms);
    }
}

impl HttpObservation {
    fn emit_completion(&self, level: Level, outcome: &str, latency_ms: f64) {
        // Keep fields on the event even when filtering disables its INFO span.
        // Noncapturing emitters retain a static tracing callsite for each level.
        macro_rules! emitter {
            ($level:expr) => {
                |observation: &Self, outcome: &str, latency_ms: f64| {
                    tracing::event!(
                        target: "batter",
                        parent: &observation.context,
                        $level,
                        request_id = observation.correlation.as_ref().map(CorrelationId::as_str),
                        method = observation.method,
                        route = observation.route.as_ref().map(MatchedPath::as_str).unwrap_or("<unmatched>"),
                        status = observation.status.map(|status| status.as_u16()),
                        http_outcome = outcome,
                        latency_ms,
                        "HTTP response boundary finished"
                    );
                }
            };
        }
        let emit: fn(&Self, &str, f64) = match level {
            Level::ERROR => emitter!(Level::ERROR),
            Level::WARN => emitter!(Level::WARN),
            Level::INFO => emitter!(Level::INFO),
            Level::DEBUG => emitter!(Level::DEBUG),
            Level::TRACE => emitter!(Level::TRACE),
        };
        emit(self, outcome, latency_ms);
    }
}
