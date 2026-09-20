//! Response facts and tracing lifetime, independent of admission policy.

#[cfg(test)]
mod tests;

use super::{CorrelationId, HttpObservationLevel};
use axum::{
    extract::{MatchedPath, Request},
    http::{Method, StatusCode},
    response::Response,
};
use std::{
    future::Future,
    sync::{Arc, Mutex},
};
use tokio::time::Instant;
use tracing::{Instrument, Level};

/// Shared facts owned by the outermost HTTP observer.
///
/// Its presence is only the single-observer marker. Operational ownership uses
/// a distinct extension, so an observer outside [`crate::operational_http`]
/// cannot suppress correlation setup. Nested Batter middleware publishes facts
/// here without taking ownership of a second completion event.
#[derive(Clone, Default)]
pub(super) struct ObservationState(Arc<Mutex<ObservationFacts>>);

#[derive(Clone, Default)]
struct ObservationFacts {
    quota: Option<crate::quota_observation::QuotaObservation>,
    correlation: Option<CorrelationId>,
}

impl ObservationState {
    fn new(
        quota: Option<crate::quota_observation::QuotaObservation>,
        correlation: Option<CorrelationId>,
    ) -> Self {
        Self(Arc::new(Mutex::new(ObservationFacts {
            quota,
            correlation,
        })))
    }

    pub(super) fn replace_quota(&self, quota: crate::quota_observation::QuotaObservation) {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .quota = Some(quota);
    }

    pub(super) fn replace_correlation(&self, correlation: CorrelationId) {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .correlation = Some(correlation);
    }

    fn facts(&self) -> ObservationFacts {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

pub(super) async fn observe_response<F: Future<Output = Response>>(
    mut request: Request,
    run: impl FnOnce(Request) -> F,
) -> Response {
    if request.extensions().get::<ObservationState>().is_some() {
        return run(request).await;
    }
    let state = ObservationState::new(
        request
            .extensions()
            .get::<crate::quota_observation::QuotaObservation>()
            .cloned(),
        request.extensions().get::<CorrelationId>().cloned(),
    );
    request.extensions_mut().insert(state.clone());
    let mut observation = HttpObservation::new(&request, state);
    let span = observation.context.clone();
    let response = run(request).instrument(span).await;
    observation.status = Some(response.status());
    observation.level = response.extensions().get::<HttpObservationLevel>().copied();
    response
}

struct HttpObservation {
    state: ObservationState,
    span: tracing::Span,
    context: tracing::Span,
    method: &'static str,
    route: Option<MatchedPath>,
    started: Instant,
    status: Option<StatusCode>,
    level: Option<HttpObservationLevel>,
}

impl HttpObservation {
    fn new(request: &Request, state: ObservationState) -> Self {
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
            state,
            span,
            context,
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
                    let facts = observation.state.facts();
                    let quota = facts.quota.map(|record| record.snapshot());
                    tracing::event!(
                        target: "batter",
                        parent: &observation.context,
                        $level,
                        request_id = facts.correlation.as_ref().map(CorrelationId::as_str),
                        method = observation.method,
                        route = observation.route.as_ref().map(MatchedPath::as_str).unwrap_or("<unmatched>"),
                        status = observation.status.map(|status| status.as_u16()),
                        http_outcome = outcome,
                        latency_ms,
                        quota_outcome = quota.map(|facts| facts.outcome()),
                        quota_consumption = quota.map(|facts| facts.consumption().label()),
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
