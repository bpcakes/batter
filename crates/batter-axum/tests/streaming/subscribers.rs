//! Which subscriber an owned body's completion event reaches: the owner of
//! the request's span before the one it was admitted under, and its own when
//! a layer copied in another request's observation record whose observer has
//! finished by the time the body starts. The library's records of a finished
//! request, and an ended body, keep no subscriber alive.

use super::support::{SECOND, assert_finished, budget, drain, finished, paused_with, ready};
use crate::capture::Capture;
use axum::{
    Router,
    body::Body,
    extract::Request,
    http::{Extensions, StatusCode},
    middleware::{self, Next},
    response::Response,
    routing::get as route,
};
use batter_axum::{
    AdmittedRequest, RequestPolicy, ResponseConstructionBudget,
    low_level::{observe_http, operational_http, request_admission},
};
use futures_util::future::BoxFuture;
use std::sync::{Arc, Mutex};
use tokio::sync::Notify;
use tower::ServiceExt;
use tracing::instrument::WithSubscriber;

type Copied = Arc<Mutex<Option<Extensions>>>;

/// Whether every strong handle to `subscriber` is gone, waiting a bounded
/// time for it. `tracing-core` briefly upgrades every registered subscriber
/// whenever a thread registers a dispatcher or a callsite and rebuilds
/// interest, so a test running in parallel can hold one for a moment. A
/// subscriber the object under test really keeps stays alive throughout.
fn released(subscriber: &tracing::dispatcher::WeakDispatch) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if subscriber.upgrade().is_none() {
            return true;
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

fn request() -> Request {
    Request::builder().uri("/").body(Body::empty()).unwrap()
}

/// A router observed by `observe_http` alone whose handler is `handler`, with
/// a layer inside the observer that copies the request's extensions, and
/// with them the observation record, into `copied`.
fn observed_and_copied<H, T>(copied: &Copied, handler: H) -> Router
where
    H: axum::handler::Handler<T, ()>,
    T: 'static,
{
    let copied = copied.clone();
    Router::new()
        .route("/", route(handler))
        .layer(middleware::from_fn(move |request: Request, next: Next| {
            *copied.lock().unwrap() = Some(request.extensions().clone());
            next.run(request)
        }))
        .layer(middleware::from_fn(observe_http))
}

/// What a layer between the operational boundary and admission does.
type Between = Arc<dyn Fn(Request, Next) -> BoxFuture<'static, Response> + Send + Sync>;

/// A router behind the operational boundary and admission whose handler is
/// `handler`, with `between` as a layer between the two.
fn admitted<H, T>(handler: H, between: Between) -> Router
where
    H: axum::handler::Handler<T, ()>,
    T: 'static,
{
    let policy = RequestPolicy::new(
        ready().operation_admission(),
        ResponseConstructionBudget::new(SECOND).unwrap(),
    );
    Router::new()
        .route("/", route(handler))
        .route_layer(middleware::from_fn_with_state(policy, request_admission))
        .layer(middleware::from_fn(move |request: Request, next: Next| {
            between(request, next)
        }))
        .layer(middleware::from_fn(operational_http))
}

/// [`admitted`] with a layer that, before admission, replaces what the
/// request's extensions hold with what `copied` holds, type by type.
fn admitted_under_copied<H, T>(copied: &Copied, handler: H) -> Router
where
    H: axum::handler::Handler<T, ()>,
    T: 'static,
{
    let copied = copied.clone();
    let overwrite: Between = Arc::new(move |mut request, next| {
        let copied = copied.lock().unwrap().clone().unwrap();
        request.extensions_mut().extend(copied);
        Box::pin(next.run(request))
    });
    admitted(handler, overwrite)
}

/// A layer that changes nothing.
fn untouched() -> Between {
    Arc::new(|request, next| Box::pin(next.run(request)))
}

/// Records the name of the span that is current, in its own subscriber, when
/// a body's completion event arrives.
struct CurrentAtTheBodyEvent(Arc<Mutex<Option<Option<&'static str>>>>);

impl<S> tracing_subscriber::Layer<S> for CurrentAtTheBodyEvent
where
    S: tracing::Subscriber + for<'lookup> tracing_subscriber::registry::LookupSpan<'lookup>,
{
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        context: tracing_subscriber::layer::Context<'_, S>,
    ) {
        if event.metadata().fields().field("body_outcome").is_some() {
            let current = context.lookup_current().map(|span| span.name());
            *self.0.lock().unwrap() = Some(current);
        }
    }
}

/// A subscriber can select events by the span that is current when they
/// occur, as a span-scoped filter does. The body's completion event is
/// emitted with the request span entered, not only parented to it, so that
/// span is the current one when the event arrives.
#[test]
fn a_body_event_is_emitted_inside_the_entered_request_span() {
    use tracing_subscriber::layer::SubscriberExt;
    let seen = Arc::new(Mutex::new(None));
    let dispatch = crate::capture::test_dispatch::new(
        tracing_subscriber::registry().with(CurrentAtTheBodyEvent(seen.clone())),
    );
    let text = paused_with(&dispatch, async {
        let app = admitted(
            |admitted: AdmittedRequest| async move {
                admitted.into_streaming_response(budget(60, 5), |_scope| "hello")
            },
            untouched(),
        );
        let mut response = app.oneshot(request()).await.unwrap();
        drain(response.body_mut()).await.0
    });
    assert_eq!(text, "hello");
    assert_eq!(*seen.lock().unwrap(), Some(Some("batter.http")));
}

/// The request's span names its subscriber, and that one receives the body's
/// event, inside the span, even where a layer polls admission and everything
/// behind it under another subscriber: the event and the span it is parented
/// to never part.
#[test]
fn a_body_reports_to_the_owner_of_its_requests_span_not_to_where_it_was_admitted() {
    let observed = Capture::new();
    let inner = Capture::new();
    let text = paused_with(&observed.dispatch, async {
        let under = inner.dispatch.clone();
        let switched: Between = Arc::new(move |request, next| {
            Box::pin(next.run(request).with_subscriber(under.clone()))
        });
        let app = admitted(
            |admitted: AdmittedRequest| async move {
                admitted.into_streaming_response(budget(60, 5), |_scope| "hello")
            },
            switched,
        );
        let mut response = app.oneshot(request()).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        drain(response.body_mut()).await.0
    });
    assert_eq!(text, "hello");
    let event = assert_finished(&observed, "INFO", "completed");
    assert!(event.contains("batter.http{"), "{event}");
    assert!(finished(&inner).is_empty(), "{}", inner.text());
}

/// A layer can hand a later request the extensions of an earlier one, and
/// with them the record of an observer that already finished. That observer
/// never saw the later request, and its record names neither a span nor a
/// subscriber any more: the later request's body event goes to the
/// subscriber that request was admitted under.
#[test]
fn a_body_admitted_under_a_finished_observation_reports_to_its_own_subscriber() {
    let earlier = Capture::new();
    let later = Capture::new();
    let copied = Copied::default();

    paused_with(&earlier.dispatch, async {
        let app = observed_and_copied(&copied, || async { "earlier" });
        assert_eq!(
            app.oneshot(request()).await.unwrap().status(),
            StatusCode::OK
        );
    });
    assert!(earlier.text().contains("HTTP response boundary finished"));

    let text = paused_with(&later.dispatch, async {
        let app = admitted_under_copied(&copied, |admitted: AdmittedRequest| async move {
            admitted.into_streaming_response(budget(60, 5), |_scope| "hello")
        });
        let mut response = app.oneshot(request()).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        drain(response.body_mut()).await.0
    });
    assert_eq!(text, "hello");
    let event = assert_finished(&later, "INFO", "completed");
    assert!(event.contains("bytes=5"), "{event}");
    assert!(finished(&earlier).is_empty(), "{}", earlier.text());
}

/// The copied record's observer can also still be alive when the later
/// request is admitted: the two requests overlap. Once both have finished,
/// that record names nothing, so a start on the later request's kept
/// admission, refused because its response was already constructed, reports
/// with that request's id to the subscriber it was admitted under: not to
/// the earlier request's, and not to the one current at the call.
#[test]
fn a_late_refusal_under_an_overlapping_requests_observation_reports_to_its_own_subscriber() {
    let earlier = Capture::new();
    let later = Capture::new();
    let elsewhere = Capture::new();
    let copied = Copied::default();
    let kept: Arc<Mutex<Option<AdmittedRequest>>> = Arc::default();
    let nobody = tracing::Dispatch::none();

    paused_with(&nobody, async {
        let entered = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let (reached, held) = (entered.clone(), release.clone());
        let first = observed_and_copied(&copied, move || async move {
            reached.notify_one();
            held.notified().await;
            "earlier"
        });
        let first = tokio::spawn(
            first
                .oneshot(request())
                .with_subscriber(earlier.dispatch.clone()),
        );
        entered.notified().await;

        // Admitted while the earlier request's observer is alive.
        let stash = kept.clone();
        let second = admitted_under_copied(&copied, move |admitted: AdmittedRequest| async move {
            *stash.lock().unwrap() = Some(admitted);
            "later"
        });
        let second = second
            .oneshot(request())
            .with_subscriber(later.dispatch.clone())
            .await
            .unwrap();
        assert_eq!(second.status(), StatusCode::OK);

        release.notify_one();
        assert_eq!(first.await.unwrap().unwrap().status(), StatusCode::OK);
    });

    let stale = kept.lock().unwrap().take().unwrap();
    let request_id = stale.correlation_id().as_str().to_owned();
    tracing::dispatcher::with_default(&elsewhere.dispatch, || {
        drop(stale.into_streaming_response(budget(60, 5), |_scope| Body::empty()));
    });
    let event = assert_finished(&later, "WARN", "cancelled");
    assert!(
        event.contains(&format!("request_id=\"{request_id}\"")),
        "{event}"
    );
    assert!(finished(&earlier).is_empty(), "{}", earlier.text());
    assert!(finished(&elsewhere).is_empty(), "{}", elsewhere.text());
}

/// An observation record is shared request metadata a layer may keep, and it
/// holds no subscriber: once the request finished and every other handle to
/// its subscriber is gone, extensions kept from it keep nothing alive, such
/// as that subscriber's output.
#[test]
fn an_observation_record_kept_beyond_its_request_keeps_no_subscriber() {
    let capture = Capture::new();
    let subscriber = capture.dispatch.downgrade();
    let copied = Copied::default();
    paused_with(&capture.dispatch, async {
        let app = observed_and_copied(&copied, || async { "observed" });
        assert_eq!(
            app.oneshot(request()).await.unwrap().status(),
            StatusCode::OK
        );
    });
    assert!(subscriber.upgrade().is_some());
    drop(capture);
    assert!(copied.lock().unwrap().is_some());
    assert!(released(&subscriber));
}

/// Nor does an admitted request or its extensions, which carry the private
/// admission record: kept beyond the response, they hold the subscriber the
/// request was admitted under only weakly. Once that subscriber is gone, a
/// start then refused has nowhere of its own left to report and reports to
/// the subscriber current at the call.
#[test]
fn an_admitted_request_and_its_extensions_kept_beyond_the_response_keep_no_subscriber() {
    let capture = Capture::new();
    let elsewhere = Capture::new();
    let subscriber = capture.dispatch.downgrade();
    let kept: Arc<Mutex<Option<(AdmittedRequest, Extensions)>>> = Arc::default();
    paused_with(&capture.dispatch, async {
        let stash = kept.clone();
        let app = admitted(
            move |admitted: AdmittedRequest, request: Request| async move {
                *stash.lock().unwrap() = Some((admitted, request.extensions().clone()));
                "kept"
            },
            untouched(),
        );
        assert_eq!(
            app.oneshot(request()).await.unwrap().status(),
            StatusCode::OK
        );
    });
    assert!(subscriber.upgrade().is_some());
    drop(capture);
    let (stale, _extensions) = kept.lock().unwrap().take().unwrap();
    assert!(released(&subscriber));

    let request_id = stale.correlation_id().as_str().to_owned();
    tracing::dispatcher::with_default(&elsewhere.dispatch, || {
        drop(stale.into_streaming_response(budget(60, 5), |_scope| Body::empty()));
    });
    let event = assert_finished(&elsewhere, "WARN", "cancelled");
    assert!(
        event.contains(&format!("request_id=\"{request_id}\"")),
        "{event}"
    );
}

/// A body that ended holds no dispatcher: neither the request's, released
/// with its span, nor the one the handler handed it over under. A holder that
/// drained it and keeps the exhausted response keeps neither subscriber
/// alive through it, whether the handler stayed with the request's
/// subscriber, as handlers ordinarily do, or switched to another.
#[test]
fn an_exhausted_body_kept_by_its_holder_keeps_no_subscriber() {
    for switches in [false, true] {
        let requested = Capture::new();
        let other = Capture::new();
        let subscribers = [requested.dispatch.downgrade(), other.dispatch.downgrade()];
        let response = paused_with(&requested.dispatch, async {
            let under = switches.then(|| other.dispatch.clone());
            let app = admitted(
                move |admitted: AdmittedRequest| async move {
                    let hand_over =
                        || admitted.into_streaming_response(budget(60, 5), |_scope| "hello");
                    match &under {
                        Some(under) => tracing::dispatcher::with_default(under, hand_over),
                        None => hand_over(),
                    }
                },
                untouched(),
            );
            let mut response = app.oneshot(request()).await.unwrap();
            assert_eq!(drain(response.body_mut()).await.0, "hello");
            response
        });
        assert_finished(&requested, "INFO", "completed");
        drop((requested, other));
        for subscriber in subscribers {
            assert!(released(&subscriber), "switches: {switches}");
        }
        drop(response);
    }
}

/// An observer whose subscriber enables no span publishes none, so there is
/// no span to name the request's subscriber even while that observer is
/// alive. A failing body's WARN event then goes to the subscriber the request
/// was admitted under, here another one than the observer's, which a layer
/// polls admission under.
#[test]
fn a_body_without_a_request_span_reports_to_where_its_request_was_admitted() {
    let observed = Capture::with_max_level(tracing::Level::WARN);
    let inner = Capture::with_max_level(tracing::Level::WARN);
    paused_with(&observed.dispatch, async {
        let under = inner.dispatch.clone();
        let switched: Between = Arc::new(move |request, next| {
            Box::pin(next.run(request).with_subscriber(under.clone()))
        });
        let app = admitted(
            |admitted: AdmittedRequest| async move {
                admitted.into_streaming_response(budget(60, 5), |_scope| {
                    let failed = std::io::Error::other("source failed");
                    Body::from_stream(futures_util::stream::iter([Err::<axum::body::Bytes, _>(
                        failed,
                    )]))
                })
            },
            switched,
        );
        let mut response = app.oneshot(request()).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(drain(response.body_mut()).await.1.is_some());
    });
    let event = assert_finished(&inner, "WARN", "failed");
    assert!(!event.contains("batter.http{"), "{event}");
    assert!(finished(&observed).is_empty(), "{}", observed.text());
}
