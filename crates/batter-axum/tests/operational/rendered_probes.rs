//! Application-rendered probes: the renderer receives every readiness decision
//! and chooses the body, while the boundary keeps the status, decision and
//! severity, and both probes stay outside admission.

use crate::capture::Capture;
use axum::{
    Extension, Router,
    body::{Body, to_bytes},
    http::{Method, Request, StatusCode, header, request::Parts},
    response::{IntoResponse, Response},
    routing::get,
};
use batter_axum::{
    CorrelationId, GuardedRouter, HttpBoundary, HttpObservationLevel, ProbePath, ReadinessDecision,
    ReadinessPolicy, RequestInterruptionResponder, RequestPolicy, ResponseConstructionBudget,
    default_readiness_level, readiness_status,
};
use batter_core::{
    cleanup::CleanupBudget,
    health::{DependencyUnreadyReason, HealthMonitor, HealthPolicy},
    lifecycle::{ShutdownBudget, ShutdownHandle, Supervisor},
    operation::OperationContext,
    readiness::{ReadinessCondition, ReadinessUnreadyReason},
};
use std::{
    future::{Future, poll_fn},
    io,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    task::Poll,
    time::Duration,
};
use tower::ServiceExt;
use tracing::{Level, instrument::WithSubscriber};

const SECOND: Duration = Duration::from_secs(1);
const TEXT: &str = "text/plain; charset=utf-8";

/// Every decision a readiness renderer received, in request order.
type Rendered = Arc<Mutex<Vec<ReadinessDecision>>>;

/// An application condition and how often readiness asked it.
#[derive(Clone, Default)]
struct Leases {
    valid: Arc<AtomicBool>,
    asked: Arc<AtomicUsize>,
}

impl Leases {
    fn condition() -> ReadinessCondition {
        ReadinessCondition::new("key-leases").unwrap()
    }

    fn attach(&self, policy: ReadinessPolicy<io::Error>) -> ReadinessPolicy<io::Error> {
        let (valid, asked) = (self.valid.clone(), self.asked.clone());
        policy.with_condition(Self::condition(), move || {
            asked.fetch_add(1, Ordering::SeqCst);
            valid.load(Ordering::SeqCst)
        })
    }

    fn set(&self, valid: bool) {
        self.valid.store(valid, Ordering::SeqCst);
    }

    fn asked(&self) -> usize {
        self.asked.load(Ordering::SeqCst)
    }
}

/// What the request metadata handed to a renderer contains.
fn metadata(parts: &Parts) -> String {
    format!(
        "correlated={};admitted={};responder={}",
        parts.extensions.get::<CorrelationId>().is_some(),
        parts.extensions.get::<OperationContext>().is_some(),
        parts
            .extensions
            .get::<RequestInterruptionResponder>()
            .is_some(),
    )
}

/// A renderer that tries to alter the decision: it always answers 200 with a
/// forged Ready decision and TRACE severity.
fn contradicting(
    rendered: &Rendered,
) -> impl Fn(ReadinessDecision, &Parts) -> Response + Send + Sync + 'static {
    let rendered = rendered.clone();
    move |decision, parts| {
        rendered.lock().unwrap().push(decision);
        (
            StatusCode::OK,
            Extension(ReadinessDecision::Ready),
            Extension(HttpObservationLevel(Level::TRACE)),
            [(header::CONTENT_TYPE, TEXT)],
            format!("decision={decision:?};{}", metadata(parts)),
        )
            .into_response()
    }
}

async fn rendered_app(
    admission: &ShutdownHandle,
    policy: ReadinessPolicy<io::Error>,
    rendered: &Rendered,
) -> Router {
    let budget = ResponseConstructionBudget::new(SECOND).unwrap();
    HttpBoundary::new(RequestPolicy::new(admission.operation_admission(), budget))
        .with_rendered_readiness(
            ProbePath::new("/ready").unwrap(),
            policy,
            contradicting(rendered),
        )
        .unwrap()
        .assemble(GuardedRouter::new().route("/work", get(|| async { "work" })))
        .await
        .unwrap()
        .into_router()
}

fn get_request(path: &str) -> Request<Body> {
    Request::builder()
        .method(Method::GET)
        .uri(path)
        .body(Body::empty())
        .unwrap()
}

async fn text(response: Response) -> String {
    String::from_utf8(to_bytes(response.into_body(), 4096).await.unwrap().to_vec()).unwrap()
}

/// Probe readiness and check that the renderer received `expected` and chose
/// the body, while the boundary chose the status, decision and severity.
async fn assert_rendered(
    app: &Router,
    policy: &ReadinessPolicy<io::Error>,
    rendered: &Rendered,
    expected: ReadinessDecision,
) {
    assert_eq!(policy.decision(), expected);
    let response = app.clone().oneshot(get_request("/ready")).await.unwrap();
    assert_eq!(rendered.lock().unwrap().last(), Some(&expected));
    assert_eq!(response.status(), readiness_status(expected));
    assert_eq!(
        response.extensions().get::<ReadinessDecision>(),
        Some(&expected)
    );
    assert_eq!(
        response
            .extensions()
            .get::<HttpObservationLevel>()
            .unwrap()
            .0,
        default_readiness_level(expected)
    );
    assert_eq!(response.headers()[header::CONTENT_TYPE], TEXT);
    assert!(response.headers().contains_key("x-request-id"));
    assert_eq!(
        text(response).await,
        format!("decision={expected:?};correlated=true;admitted=false;responder=false")
    );
}

/// Neither answer of the application condition changes an unready lifecycle
/// or dependency decision.
async fn assert_unready_either_way(
    app: &Router,
    policy: &ReadinessPolicy<io::Error>,
    rendered: &Rendered,
    leases: &Leases,
    expected: ReadinessDecision,
) {
    for valid in [true, false] {
        leases.set(valid);
        assert_rendered(app, policy, rendered, expected).await;
    }
    leases.set(true);
}

async fn sample(run: &mut (impl Future<Output = ()> + Unpin)) {
    assert!(
        poll_fn(|cx| Poll::Ready(std::pin::Pin::new(&mut *run).poll(cx)))
            .await
            .is_pending()
    );
}

const fn unready(reason: ReadinessUnreadyReason) -> ReadinessDecision {
    ReadinessDecision::Unready(reason)
}

const fn dependency(reason: DependencyUnreadyReason) -> ReadinessDecision {
    unready(ReadinessUnreadyReason::Dependency(reason))
}

/// Every decision the renderer must receive. The exhaustive match fails to
/// compile when a variant is added without extending this list.
fn every_decision() -> Vec<ReadinessDecision> {
    let decisions = vec![
        ReadinessDecision::Ready,
        unready(ReadinessUnreadyReason::Starting),
        unready(ReadinessUnreadyReason::Draining),
        unready(ReadinessUnreadyReason::Stopped),
        dependency(DependencyUnreadyReason::Unknown),
        dependency(DependencyUnreadyReason::ProbeFailed),
        dependency(DependencyUnreadyReason::ProbeTimedOut),
        dependency(DependencyUnreadyReason::Stale),
        dependency(DependencyUnreadyReason::WriterStopped),
        unready(ReadinessUnreadyReason::Condition(Leases::condition())),
    ];
    for decision in &decisions {
        match decision {
            ReadinessDecision::Ready
            | ReadinessDecision::Unready(
                ReadinessUnreadyReason::Starting
                | ReadinessUnreadyReason::Draining
                | ReadinessUnreadyReason::Stopped
                | ReadinessUnreadyReason::Condition(_)
                | ReadinessUnreadyReason::Dependency(
                    DependencyUnreadyReason::Unknown
                    | DependencyUnreadyReason::ProbeFailed
                    | DependencyUnreadyReason::ProbeTimedOut
                    | DependencyUnreadyReason::Stale
                    | DependencyUnreadyReason::WriterStopped,
                ),
            ) => {}
        }
    }
    decisions
}

#[tokio::test(start_paused = true)]
async fn renderer_receives_every_decision_without_choosing_status_decision_or_severity() {
    let capture = Capture::with_max_level(Level::TRACE);
    let rendered = Rendered::default();
    async {
        let calls = Arc::new(AtomicUsize::new(0));
        let attempts = calls.clone();
        let monitor = HealthMonitor::new(
            HealthPolicy::new(SECOND, 2 * SECOND, 4 * SECOND, SECOND).unwrap(),
            move || {
                let attempt = attempts.fetch_add(1, Ordering::SeqCst);
                async move {
                    match attempt {
                        0 => Err(io::Error::other("secret-dependency-cause")),
                        1 => std::future::pending().await,
                        _ => Ok(()),
                    }
                }
            },
        );
        let leases = Leases::default();
        let (handle, approval) = ShutdownHandle::new_with_readiness_approval();
        let policy = leases.attach(ReadinessPolicy::new(handle.status(), monitor.reader()));
        let app = rendered_app(&handle, policy.clone(), &rendered).await;
        let probe =
            |expected| assert_unready_either_way(&app, &policy, &rendered, &leases, expected);

        probe(unready(ReadinessUnreadyReason::Starting)).await;
        approval.approve();
        probe(dependency(DependencyUnreadyReason::Unknown)).await;
        let probe_handle = ShutdownHandle::new_unapproved();
        let mut run = Box::pin(monitor.run(probe_handle.signal()));
        sample(&mut run).await;
        probe(dependency(DependencyUnreadyReason::ProbeFailed)).await;
        tokio::time::advance(2 * SECOND).await;
        sample(&mut run).await;
        tokio::time::advance(SECOND).await;
        sample(&mut run).await;
        probe(dependency(DependencyUnreadyReason::ProbeTimedOut)).await;
        // The condition is never asked while the dependency is unready.
        assert_eq!(leases.asked(), 0);

        tokio::time::advance(2 * SECOND).await;
        sample(&mut run).await;
        assert_rendered(&app, &policy, &rendered, ReadinessDecision::Ready).await;
        // The application condition turns the ready decision unready, then back.
        leases.set(false);
        let narrowed = unready(ReadinessUnreadyReason::Condition(Leases::condition()));
        assert_rendered(&app, &policy, &rendered, narrowed).await;
        leases.set(true);
        assert_rendered(&app, &policy, &rendered, ReadinessDecision::Ready).await;
        assert!(leases.asked() > 0);

        tokio::time::advance(4 * SECOND).await;
        probe(dependency(DependencyUnreadyReason::Stale)).await;
        drop(run);
        probe(dependency(DependencyUnreadyReason::WriterStopped)).await;
        handle.request();
        probe(unready(ReadinessUnreadyReason::Draining)).await;
        assert_eq!(calls.load(Ordering::SeqCst), 3);

        let cleanup = CleanupBudget::new(SECOND, SECOND, SECOND).unwrap();
        let supervisor =
            Supervisor::new(ShutdownBudget::new(SECOND, SECOND, SECOND, cleanup).unwrap());
        let healthy = HealthMonitor::new(
            HealthPolicy::new(SECOND, 2 * SECOND, 4 * SECOND, SECOND).unwrap(),
            || async { Ok::<_, io::Error>(()) },
        );
        let stopped = leases.attach(ReadinessPolicy::new(supervisor.status(), healthy.reader()));
        let stopped_app = rendered_app(&handle, stopped.clone(), &rendered).await;
        let report = supervisor.start().wait().await.unwrap();
        assert!(!report.is_success()); // Empty supervisor, used only to observe terminal state.
        let stopped_decision = unready(ReadinessUnreadyReason::Stopped);
        assert_unready_either_way(&stopped_app, &stopped, &rendered, &leases, stopped_decision)
            .await;
    }
    .with_subscriber(capture.dispatch.clone())
    .await;

    let rendered = rendered.lock().unwrap().clone();
    for decision in every_decision() {
        assert!(
            rendered.contains(&decision),
            "{decision:?} was not rendered"
        );
    }
    // One completion event per probe, at the policy's severity rather than the
    // renderer's forged TRACE.
    let text = capture.text();
    let events: Vec<_> = text
        .lines()
        .filter(|line| line.contains("HTTP response boundary finished"))
        .collect();
    assert_eq!(events.len(), rendered.len(), "{text}");
    for (event, decision) in events.iter().zip(&rendered) {
        let level = default_readiness_level(*decision).as_str();
        assert!(event.trim_start().starts_with(level), "{event}");
        let status = readiness_status(*decision).as_u16();
        assert!(event.contains(&format!("status={status}")), "{event}");
    }
    assert!(!text.contains("secret-dependency-cause"));
}

#[tokio::test]
async fn rendered_probes_answer_outside_admission_with_application_bodies() {
    let (handle, approval) = ShutdownHandle::new_with_readiness_approval();
    let monitor = HealthMonitor::new(
        HealthPolicy::new(SECOND, SECOND, 3 * SECOND, SECOND).unwrap(),
        || async { Ok::<_, io::Error>(()) },
    );
    let work = Arc::new(AtomicUsize::new(0));
    let worked = work.clone();
    let budget = ResponseConstructionBudget::new(SECOND).unwrap();
    let app = HttpBoundary::new(RequestPolicy::new(handle.operation_admission(), budget))
        .with_rendered_liveness(ProbePath::new("/live").unwrap(), |parts| {
            // Liveness status is not the renderer's to choose either.
            (
                StatusCode::SERVICE_UNAVAILABLE,
                [(header::CONTENT_TYPE, TEXT)],
                format!("live;{}", metadata(parts)),
            )
                .into_response()
        })
        .unwrap()
        .with_rendered_readiness(
            ProbePath::new("/ready").unwrap(),
            ReadinessPolicy::new(handle.status(), monitor.reader()),
            |decision, parts| {
                format!("ready={};{}", decision.is_ready(), metadata(parts)).into_response()
            },
        )
        .unwrap()
        .assemble(GuardedRouter::new().route(
            "/work",
            get(move || {
                worked.fetch_add(1, Ordering::SeqCst);
                async { "work" }
            }),
        ))
        .await
        .unwrap()
        .into_router();
    let call = |path: &'static str| {
        let app = app.clone();
        async move {
            let response = app.oneshot(get_request(path)).await.unwrap();
            assert!(response.headers().contains_key("x-request-id"));
            (response.status(), text(response).await)
        }
    };
    // Admission rejects guarded work before its handler; both probes answer
    // with their application bodies and are never admitted.
    let assert_unadmitted_probes = || async {
        let (status, body) = call("/work").await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert!(body.contains("service_unavailable"), "{body}");
        let unadmitted = "correlated=true;admitted=false;responder=false";
        assert_eq!(
            call("/live").await,
            (StatusCode::OK, format!("live;{unadmitted}"))
        );
        assert_eq!(
            call("/ready").await,
            (
                StatusCode::SERVICE_UNAVAILABLE,
                format!("ready=false;{unadmitted}")
            )
        );
    };

    assert_unadmitted_probes().await;
    approval.approve();
    assert_eq!(call("/work").await, (StatusCode::OK, "work".to_owned()));
    handle.request();
    assert_unadmitted_probes().await;
    assert_eq!(work.load(Ordering::SeqCst), 1);
    drop(monitor);
}
