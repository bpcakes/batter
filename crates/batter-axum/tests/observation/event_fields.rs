use super::{
    capture::Capture,
    support::{Boundary, request},
};
use axum::{
    Extension, Router,
    http::StatusCode,
    middleware,
    routing::{any, get},
};
use batter::lifecycle::ShutdownHandle;
use batter_axum::{HttpObservationLevel, RequestPolicy, observe_http};
use std::{
    collections::BTreeMap,
    fmt,
    future::{Future, poll_fn},
    sync::{Arc, Mutex},
    task::Poll,
    time::Duration,
};
use tower::ServiceExt;
use tracing::{
    Dispatch, Event, Level, Subscriber,
    field::{Field, Visit},
    instrument::WithSubscriber,
};
use tracing_subscriber::{Layer, filter::LevelFilter, layer::Context, prelude::*};

#[derive(Debug, PartialEq)]
enum Value {
    Text(String),
    Integer(u64),
    Float(f64),
}

#[derive(Debug, Default)]
struct Fields(BTreeMap<String, Value>);

impl Visit for Fields {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.0
            .insert(field.name().into(), Value::Text(value.into()));
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.0.insert(field.name().into(), Value::Integer(value));
    }

    fn record_f64(&mut self, field: &Field, value: f64) {
        self.0.insert(field.name().into(), Value::Float(value));
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.0
            .insert(field.name().into(), Value::Text(format!("{value:?}")));
    }
}

#[derive(Debug)]
struct Completion {
    level: Level,
    has_parent: bool,
    fields: Fields,
}

#[derive(Clone, Default)]
struct Events(Arc<Mutex<Vec<Completion>>>);

impl<S: Subscriber> Layer<S> for Events {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        let mut fields = Fields::default();
        // Only read this event's fields: span formatting cannot satisfy the oracle.
        event.record(&mut fields);
        if event.metadata().target() == "batter"
            && fields.0.get("message")
                == Some(&Value::Text("HTTP response boundary finished".into()))
        {
            self.0.lock().unwrap().push(Completion {
                level: *event.metadata().level(),
                has_parent: event.parent().is_some(),
                fields,
            });
        }
    }
}

impl Events {
    fn dispatch(&self, max_level: Level) -> Dispatch {
        super::capture::retain_dispatch(Dispatch::new(
            tracing_subscriber::registry()
                .with(LevelFilter::from_level(max_level))
                .with(self.clone()),
        ))
    }

    fn assert_completion(
        &self,
        level: Level,
        status: Option<u16>,
        outcome: &str,
        route: &str,
        method: &str,
    ) {
        let events = self.0.lock().unwrap();
        assert_eq!(events.len(), 1, "{events:?}");
        let event = &events[0];
        assert_eq!(event.level, level);
        let fields = &event.fields.0;
        for (key, expected) in [
            ("method", method),
            ("route", route),
            ("http_outcome", outcome),
        ] {
            assert_eq!(
                fields.get(key),
                Some(&Value::Text(expected.into())),
                "{events:?}"
            );
        }
        assert_eq!(
            fields.get("status"),
            status.map(|s| Value::Integer(s.into())).as_ref()
        );
        assert!(
            matches!(fields.get("latency_ms"), Some(Value::Float(ms)) if ms.is_finite() && *ms >= 0.0)
        );
        assert!(!format!("{events:?}").contains("secret-"));
        assert!(!format!("{events:?}").contains("SECRET-METHOD"));
    }
}

#[tokio::test(start_paused = true)]
async fn completion_fields_belong_to_events_at_every_level_with_independent_span_filtering() {
    for mode in [Boundary::Observation, Boundary::Split, Boundary::Combined] {
        for (level, status, outcome) in [
            (
                Level::ERROR,
                StatusCode::SERVICE_UNAVAILABLE,
                "server_error",
            ),
            (
                Level::WARN,
                StatusCode::INTERNAL_SERVER_ERROR,
                "server_error",
            ),
            (Level::INFO, StatusCode::OK, "completed"),
            (Level::DEBUG, StatusCode::NOT_FOUND, "client_error"),
            (
                Level::TRACE,
                StatusCode::SERVICE_UNAVAILABLE,
                "server_error",
            ),
        ] {
            let events = Events::default();
            let handle = ShutdownHandle::new();
            handle.mark_ready();
            let router = mode.apply(
                Router::new().route(
                    "/records/{id}",
                    any(move || async move {
                        tokio::time::sleep(Duration::from_millis(25)).await;
                        (Extension(HttpObservationLevel(level)), status)
                    }),
                ),
                RequestPolicy::new(handle, Duration::from_secs(1)).unwrap(),
            );
            let response = router
                .oneshot(request(
                    "SECRET-METHOD",
                    "/records/secret-id?secret-query=1",
                ))
                .with_subscriber(events.dispatch(level))
                .await
                .unwrap();
            assert_eq!(response.status(), status);
            events.assert_completion(
                level,
                Some(status.as_u16()),
                outcome,
                "/records/{id}",
                "OTHER",
            );
            assert_eq!(events.0.lock().unwrap()[0].has_parent, level >= Level::INFO);
            assert_eq!(
                events.0.lock().unwrap()[0].fields.0.get("latency_ms"),
                Some(&Value::Float(25.0))
            );
        }
    }
}

#[tokio::test]
async fn rejected_and_unmatched_requests_keep_event_fields_without_info_spans() {
    async fn must_not_run() -> StatusCode {
        panic!("handler must not execute")
    }

    for mode in [Boundary::Split, Boundary::Combined] {
        let events = Events::default();
        let router = mode.apply(
            Router::new().route("/work", get(must_not_run)),
            RequestPolicy::new(ShutdownHandle::new(), Duration::from_secs(1)).unwrap(),
        );
        assert_eq!(
            router
                .oneshot(request("GET", "/work"))
                .with_subscriber(events.dispatch(Level::WARN))
                .await
                .unwrap()
                .status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
        events.assert_completion(Level::WARN, Some(503), "server_error", "/work", "GET");
        assert!(!events.0.lock().unwrap()[0].has_parent);
    }

    let events = Events::default();
    let router = Router::new()
        .fallback(|| async { StatusCode::INTERNAL_SERVER_ERROR })
        .layer(middleware::from_fn(observe_http));
    assert_eq!(
        router
            .oneshot(request("GET", "/secret-unmatched?secret-query=1"))
            .with_subscriber(events.dispatch(Level::WARN))
            .await
            .unwrap()
            .status(),
        StatusCode::INTERNAL_SERVER_ERROR
    );
    events.assert_completion(Level::WARN, Some(500), "server_error", "<unmatched>", "GET");
    assert!(!events.0.lock().unwrap()[0].has_parent);
}

#[test]
fn dropped_future_keeps_event_fields_and_first_poll_dispatch_without_info_spans() {
    for mode in [Boundary::Observation, Boundary::Split, Boundary::Combined] {
        let events = Events::default();
        let ambient = Capture::new();
        ambient.block_on(async {
            let handle = ShutdownHandle::new();
            handle.mark_ready();
            let router = mode.apply(
                Router::new().route(
                    "/records/{id}",
                    get(|| async { std::future::pending::<StatusCode>().await }),
                ),
                RequestPolicy::new(handle, Duration::from_secs(60)).unwrap(),
            );
            let mut future = Box::pin(
                router
                    .oneshot(request("GET", "/records/secret-id"))
                    .with_subscriber(events.dispatch(Level::WARN)),
            );
            poll_fn(|cx| {
                assert!(future.as_mut().poll(cx).is_pending());
                Poll::Ready(())
            })
            .await;
            // WithSubscriber protects polling only; Batter must protect destruction.
            drop(future);
        });
        events.assert_completion(Level::WARN, None, "dropped", "/records/{id}", "GET");
        assert!(!events.0.lock().unwrap()[0].has_parent);
        assert!(ambient.text().is_empty(), "{}", ambient.text());
    }
}
