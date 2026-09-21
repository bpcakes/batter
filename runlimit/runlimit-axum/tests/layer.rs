//! Behavioral boundaries for the Axum admission middleware.

use std::{
    convert::Infallible,
    error::Error,
    fmt,
    future::{Future, Ready, ready},
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    task::{Context, Poll, Waker},
    time::Duration,
};

use axum::{
    Extension, Router,
    body::{Body, to_bytes},
    extract::Request,
    http::{HeaderValue, StatusCode},
    response::Response,
    routing::get,
};
use bytes::Bytes;
use http_body::{Body as HttpBody, Frame, SizeHint};
use runlimit_axum::{
    Admission, Admissions, ExtractSubjectKey, RateLimitLayer, RateLimitRejection, RejectionKind,
};
use runlimit_core::{
    Admitted, AdmittedView, Allowance, BatchDecision, Capacity, Check, Decision, Denial,
    FixedWindowPolicy, KeyHasher, Limiter, PolicyId, QuotaDenial, QuotaMode, RateLimitPolicy,
    ScopeId, SubjectKey,
};
use tower::{Layer, Service, ServiceExt, service_fn};

#[derive(Clone, Copy)]
enum StubOutcome {
    Decision(Decision),
    BackendError,
    DecisionFromPolicyMode,
}

#[derive(Clone)]
struct StubLimiter {
    calls: Arc<AtomicUsize>,
    subjects: Arc<Mutex<Vec<SubjectKey>>>,
    outcome: StubOutcome,
}

impl StubLimiter {
    fn returning(decision: Decision) -> Self {
        Self {
            calls: Arc::new(AtomicUsize::new(0)),
            subjects: Arc::new(Mutex::new(Vec::new())),
            outcome: StubOutcome::Decision(decision),
        }
    }

    fn failing() -> Self {
        Self {
            calls: Arc::new(AtomicUsize::new(0)),
            subjects: Arc::new(Mutex::new(Vec::new())),
            outcome: StubOutcome::BackendError,
        }
    }

    fn deciding_from_policy_mode() -> Self {
        Self {
            calls: Arc::new(AtomicUsize::new(0)),
            subjects: Arc::new(Mutex::new(Vec::new())),
            outcome: StubOutcome::DecisionFromPolicyMode,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct StubError;

impl fmt::Display for StubError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("stub backend failure")
    }
}

impl Error for StubError {}

impl Limiter for StubLimiter {
    type Policy = FixedWindowPolicy;
    type CheckError = StubError;
    type CheckAllError = StubError;

    fn check(
        &self,
        check: &Check<'_, Self::Policy>,
    ) -> impl Future<Output = Result<Decision, Self::CheckError>> + Send {
        self.calls.fetch_add(1, Ordering::Relaxed);
        self.subjects.lock().unwrap().push(check.subject());
        ready(match self.outcome {
            StubOutcome::Decision(decision) => Ok(decision),
            StubOutcome::BackendError => Err(StubError),
            StubOutcome::DecisionFromPolicyMode => {
                let denial = QuotaDenial::new(check.policy().capacity(), Duration::from_secs(1));
                Ok(match check.policy().quota_mode() {
                    QuotaMode::Enforce => Decision::denied(denial),
                    QuotaMode::Shadow => Decision::shadow_denied(denial),
                })
            }
        })
    }

    fn check_all(
        &self,
        _checks: &[Check<'_, Self::Policy>],
    ) -> impl Future<Output = Result<BatchDecision, Self::CheckAllError>> + Send {
        ready(Err(StubError))
    }
}

fn policy() -> FixedWindowPolicy {
    policy_for_scope("client", 8)
}

fn policy_for_scope(scope: &str, limit: u64) -> FixedWindowPolicy {
    FixedWindowPolicy::new(
        PolicyId::new("auth.login").unwrap(),
        ScopeId::new(scope).unwrap(),
        limit,
        Duration::from_mins(1),
    )
    .unwrap()
}

fn allowance(capacity: u64, available: u64) -> Allowance {
    Allowance::new(
        Capacity::new(capacity).unwrap(),
        available,
        Duration::from_mins(1),
    )
    .unwrap()
}

fn allowed(capacity: u64, available: u64) -> Decision {
    Decision::allowed(allowance(capacity, available))
}

fn quota(capacity: u64, retry_after: Duration) -> QuotaDenial {
    QuotaDenial::new(Capacity::new(capacity).unwrap(), retry_after)
}

fn subject(byte: u8) -> SubjectKey {
    SubjectKey::from_digest([byte; 32])
}

struct HashedExtractor(KeyHasher);

impl<B> ExtractSubjectKey<FixedWindowPolicy, B> for HashedExtractor {
    type Error = Infallible;

    fn extract_subject_key(
        &self,
        _request: &Request<B>,
        policy: &FixedWindowPolicy,
    ) -> Result<SubjectKey, Self::Error> {
        Ok(self
            .0
            .hash_for(policy, b"normalized-client")
            .into_unbound_subject_key())
    }
}

struct AlternatePolicyExtractor {
    hasher: KeyHasher,
    alternate_policy: FixedWindowPolicy,
}

impl<B> ExtractSubjectKey<FixedWindowPolicy, B> for AlternatePolicyExtractor {
    type Error = Infallible;

    fn extract_subject_key(
        &self,
        _request: &Request<B>,
        _configured_policy: &FixedWindowPolicy,
    ) -> Result<SubjectKey, Self::Error> {
        Ok(self
            .hasher
            .hash_for(&self.alternate_policy, b"normalized-client")
            .into_unbound_subject_key())
    }
}

fn response(status: StatusCode) -> Response {
    Response::builder()
        .status(status)
        .body(Body::empty())
        .unwrap()
}

/// Maps every rejection category without a wildcard arm.
fn map_rejection(rejection: &RateLimitRejection<&'static str, StubError>) -> Response {
    match rejection {
        RateLimitRejection::Key(_) => response(StatusCode::BAD_REQUEST),
        RateLimitRejection::Denied(_) => response(StatusCode::TOO_MANY_REQUESTS),
        RateLimitRejection::Backend(StubError) => response(StatusCode::SERVICE_UNAVAILABLE),
    }
}

fn expect_single_admission(admissions: &Admissions, policy: &FixedWindowPolicy) -> Admitted {
    assert_eq!(admissions.len(), 1);
    let admission = admissions
        .get(policy)
        .expect("the evaluated policy has an admission");
    assert_eq!(admission.policy_id(), policy.id());
    assert_eq!(admission.scope_id(), policy.scope());
    assert_eq!(admission.policy_fingerprint(), policy.fingerprint());
    admission.decision()
}

#[tokio::test]
async fn layer_composes_with_an_axum_router() {
    let decision = allowed(8, 7);
    let layer = RateLimitLayer::new(
        StubLimiter::returning(decision),
        policy(),
        |_request: &Request<Body>, _policy: &FixedWindowPolicy| Ok::<_, &'static str>(subject(0)),
        |rejection| map_rejection(&rejection),
    );
    let app = Router::new()
        .route(
            "/",
            get(
                move |Extension(admissions): Extension<Admissions>| async move {
                    assert_eq!(
                        expect_single_admission(&admissions, &policy()),
                        Admitted::allowed(allowance(8, 7))
                    );
                    StatusCode::NO_CONTENT
                },
            ),
        )
        .layer(layer);

    let result = app
        .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(result.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn named_extractor_forwards_the_exact_derived_subject() {
    let configured_policy = policy();
    let hasher = KeyHasher::new([0x42; 32]).unwrap();
    let expected_subject = hasher
        .hash_for(&configured_policy, b"normalized-client")
        .into_unbound_subject_key();
    let limiter = StubLimiter::returning(allowed(8, 7));
    let calls = Arc::clone(&limiter.calls);
    let observed_subjects = Arc::clone(&limiter.subjects);
    let layer = RateLimitLayer::new(
        limiter,
        configured_policy,
        HashedExtractor(hasher),
        |_rejection: RateLimitRejection<Infallible, StubError>| {
            panic!("the derived subject must be accepted")
        },
    );
    let inner = service_fn(|_request: Request<Body>| {
        ready(Ok::<_, Infallible>(response(StatusCode::NO_CONTENT)))
    });

    let result = layer
        .layer(inner)
        .oneshot(Request::new(Body::empty()))
        .await
        .unwrap();

    assert_eq!(result.status(), StatusCode::NO_CONTENT);
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    assert_eq!(
        observed_subjects.lock().unwrap().as_slice(),
        &[expected_subject]
    );
}

#[tokio::test]
async fn named_extractor_cannot_replace_the_layer_policy() {
    let configured_policy = policy();
    let alternate_policy = policy().with_quota_mode(QuotaMode::Shadow);
    let hasher = KeyHasher::new([0x24; 32]).unwrap();
    let expected_subject = hasher
        .hash_for(&alternate_policy, b"normalized-client")
        .into_unbound_subject_key();
    let limiter = StubLimiter::deciding_from_policy_mode();
    let observed_subjects = Arc::clone(&limiter.subjects);
    let inner_calls = Arc::new(AtomicUsize::new(0));
    let inner_calls_for_service = Arc::clone(&inner_calls);
    let inner = service_fn(move |_request: Request<Body>| {
        inner_calls_for_service.fetch_add(1, Ordering::Relaxed);
        ready(Ok::<_, Infallible>(response(StatusCode::NO_CONTENT)))
    });
    let layer = RateLimitLayer::new(
        limiter,
        configured_policy,
        AlternatePolicyExtractor {
            hasher,
            alternate_policy,
        },
        |rejection: RateLimitRejection<Infallible, StubError>| match rejection {
            RateLimitRejection::Key(never) => match never {},
            RateLimitRejection::Denied(Denial::QuotaExceeded(_)) => {
                response(StatusCode::TOO_MANY_REQUESTS)
            }
            RateLimitRejection::Denied(Denial::StorageCapacity { .. })
            | RateLimitRejection::Backend(StubError) => panic!("unexpected rejection"),
        },
    );

    let result = layer
        .layer(inner)
        .oneshot(Request::new(Body::empty()))
        .await
        .unwrap();

    assert_eq!(result.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(inner_calls.load(Ordering::Relaxed), 0);
    assert_eq!(
        observed_subjects.lock().unwrap().as_slice(),
        &[expected_subject]
    );
}

#[tokio::test]
async fn allowed_request_proceeds_with_an_admission_extension() {
    let limiter = StubLimiter::returning(allowed(8, 7));
    let inner_calls = Arc::new(AtomicUsize::new(0));
    let inner_calls_for_service = Arc::clone(&inner_calls);
    let inner = service_fn(move |request: Request<Body>| {
        let inner_calls = Arc::clone(&inner_calls_for_service);
        async move {
            inner_calls.fetch_add(1, Ordering::Relaxed);
            let admissions = request
                .extensions()
                .get::<Admissions>()
                .expect("an admitted request carries its admissions");
            assert!(matches!(
                expect_single_admission(admissions, &policy()).view(),
                AdmittedView::Allowed { allowance }
                    if allowance.capacity().get() == 8 && allowance.available() == 7
            ));
            assert!(request.extensions().get::<Decision>().is_none());
            Ok::<_, Infallible>(response(StatusCode::NO_CONTENT))
        }
    });
    let layer = RateLimitLayer::new(
        limiter,
        policy(),
        |_request: &Request<Body>, policy: &FixedWindowPolicy| {
            assert_eq!(policy.id().as_str(), "auth.login");
            Ok::<_, &'static str>(subject(1))
        },
        |_rejection| panic!("an allowed request must not be mapped"),
    );

    let result = layer
        .layer(inner)
        .oneshot(Request::new(Body::empty()))
        .await
        .unwrap();

    assert_eq!(result.status(), StatusCode::NO_CONTENT);
    assert_eq!(inner_calls.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn enforced_denial_is_mapped_and_short_circuits_inner_service() {
    let denial = quota(8, Duration::from_secs(17));
    let limiter = StubLimiter::returning(Decision::denied(denial));
    let inner_calls = Arc::new(AtomicUsize::new(0));
    let inner_calls_for_service = Arc::clone(&inner_calls);
    let inner = service_fn(move |_request: Request<Body>| {
        inner_calls_for_service.fetch_add(1, Ordering::Relaxed);
        ready(Ok::<_, Infallible>(response(StatusCode::NO_CONTENT)))
    });
    let layer = RateLimitLayer::new(
        limiter,
        policy(),
        |_request: &Request<Body>, _policy: &FixedWindowPolicy| Ok::<_, Infallible>(subject(2)),
        move |rejection: RateLimitRejection<Infallible, StubError>| {
            assert_eq!(rejection.kind(), RejectionKind::Denied);
            match rejection {
                RateLimitRejection::Key(never) => match never {},
                RateLimitRejection::Denied(mapped) => {
                    assert_eq!(mapped, Denial::QuotaExceeded(denial));
                }
                RateLimitRejection::Backend(StubError) => {
                    panic!("a denied request must not be reported as a backend failure")
                }
            }
            response(StatusCode::TOO_MANY_REQUESTS)
        },
    );

    let result = layer
        .layer(inner)
        .oneshot(Request::new(Body::empty()))
        .await
        .unwrap();

    assert_eq!(result.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(inner_calls.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn shadow_denial_proceeds_with_an_admission_extension() {
    let denial = quota(8, Duration::from_secs(17));
    let limiter = StubLimiter::returning(Decision::shadow_denied(denial));
    let inner = service_fn(move |request: Request<Body>| async move {
        let admissions = request
            .extensions()
            .get::<Admissions>()
            .expect("a shadow-denied request carries its admissions");
        assert_eq!(
            expect_single_admission(admissions, &policy()),
            Admitted::shadow_denied(denial)
        );
        Ok::<_, Infallible>(response(StatusCode::ACCEPTED))
    });
    let layer = RateLimitLayer::new(
        limiter,
        policy(),
        |_request: &Request<Body>, _policy: &FixedWindowPolicy| Ok::<_, Infallible>(subject(3)),
        |_rejection| panic!("shadow quota exhaustion must not be rejected"),
    );

    let result = layer
        .layer(inner)
        .oneshot(Request::new(Body::empty()))
        .await
        .unwrap();

    assert_eq!(result.status(), StatusCode::ACCEPTED);
}

#[tokio::test]
async fn stacked_layers_record_every_admission_in_evaluation_order() {
    let client_policy = policy_for_scope("client", 40);
    let identity_policy = policy_for_scope("identity", 8);
    let client_layer = RateLimitLayer::new(
        StubLimiter::returning(allowed(40, 39)),
        client_policy.clone(),
        |_request: &Request<Body>, _policy: &FixedWindowPolicy| Ok::<_, &'static str>(subject(11)),
        |rejection| map_rejection(&rejection),
    );
    let shadow_denial = quota(8, Duration::from_secs(3));
    let identity_layer = RateLimitLayer::new(
        StubLimiter::returning(Decision::shadow_denied(shadow_denial)),
        identity_policy.clone(),
        |_request: &Request<Body>, _policy: &FixedWindowPolicy| Ok::<_, &'static str>(subject(12)),
        |rejection| map_rejection(&rejection),
    );
    let unrelated_policy = policy_for_scope("client", 41);
    let inner = service_fn(move |request: Request<Body>| {
        let client_policy = client_policy.clone();
        let identity_policy = identity_policy.clone();
        let unrelated_policy = unrelated_policy.clone();
        async move {
            let admissions = request
                .extensions()
                .get::<Admissions>()
                .expect("stacked layers share one admissions extension");
            let recorded = admissions
                .iter()
                .map(|admission| (admission.scope_id().as_str(), admission.decision()))
                .collect::<Vec<_>>();
            assert_eq!(
                recorded,
                vec![
                    ("client", Admitted::allowed(allowance(40, 39))),
                    ("identity", Admitted::shadow_denied(shadow_denial)),
                ]
            );
            assert_eq!(
                admissions.get(&client_policy).map(Admission::decision),
                Some(Admitted::allowed(allowance(40, 39)))
            );
            assert_eq!(
                admissions.get(&identity_policy).map(Admission::decision),
                Some(Admitted::shadow_denied(shadow_denial))
            );
            assert!(
                admissions.get(&unrelated_policy).is_none(),
                "a policy with a different fingerprint has no admission"
            );
            assert_eq!(admissions.into_iter().count(), 2);
            Ok::<_, Infallible>(response(StatusCode::NO_CONTENT))
        }
    });

    // The client gate wraps the identity gate, so it is evaluated first.
    let result = client_layer
        .layer(identity_layer.layer(inner))
        .oneshot(Request::new(Body::empty()))
        .await
        .unwrap();

    assert_eq!(result.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn identical_policy_layers_keep_both_admissions_and_lookup_the_outermost() {
    let policy = policy_for_scope("shared", 8);
    let outer_admitted = Admitted::allowed(allowance(8, 7));
    let inner_denial = quota(8, Duration::from_secs(3));
    let inner_admitted = Admitted::shadow_denied(inner_denial);
    let outer = RateLimitLayer::new(
        StubLimiter::returning(Decision::from(outer_admitted)),
        policy.clone(),
        |_request: &Request<Body>, _policy: &FixedWindowPolicy| Ok::<_, &'static str>(subject(17)),
        |rejection| map_rejection(&rejection),
    );
    let inner_layer = RateLimitLayer::new(
        StubLimiter::returning(Decision::from(inner_admitted)),
        policy.clone(),
        |_request: &Request<Body>, _policy: &FixedWindowPolicy| Ok::<_, &'static str>(subject(18)),
        |rejection| map_rejection(&rejection),
    );
    let inner = service_fn(move |request: Request<Body>| {
        let policy = policy.clone();
        async move {
            let admissions = request
                .extensions()
                .get::<Admissions>()
                .expect("identical-policy layers share one admissions extension");
            let recorded = admissions
                .iter()
                .map(Admission::decision)
                .collect::<Vec<_>>();

            assert_eq!(recorded, vec![outer_admitted, inner_admitted]);
            assert_eq!(
                admissions.get(&policy).map(Admission::decision),
                Some(outer_admitted),
                "lookup returns the first, outermost matching admission"
            );
            Ok::<_, Infallible>(response(StatusCode::NO_CONTENT))
        }
    });

    let result = outer
        .layer(inner_layer.layer(inner))
        .oneshot(Request::new(Body::empty()))
        .await
        .unwrap();

    assert_eq!(result.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn inner_layer_denial_rejects_a_request_the_outer_layer_admitted() {
    let outer = RateLimitLayer::new(
        StubLimiter::returning(allowed(40, 39)),
        policy_for_scope("client", 40),
        |_request: &Request<Body>, _policy: &FixedWindowPolicy| Ok::<_, &'static str>(subject(13)),
        |_rejection| panic!("the outer layer admits every request"),
    );
    let inner_layer = RateLimitLayer::new(
        StubLimiter::returning(Decision::denied(Denial::StorageCapacity {
            retry_after: None,
        })),
        policy_for_scope("identity", 8),
        |_request: &Request<Body>, _policy: &FixedWindowPolicy| Ok::<_, &'static str>(subject(14)),
        |rejection| map_rejection(&rejection),
    );
    let inner = service_fn(|_request: Request<Body>| {
        ready(Ok::<_, Infallible>(response(StatusCode::NO_CONTENT)))
    });

    let result = outer
        .layer(inner_layer.layer(inner))
        .oneshot(Request::new(Body::empty()))
        .await
        .unwrap();

    assert_eq!(result.status(), StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn key_and_backend_failures_are_owned_by_the_mapper() {
    let unused_inner = service_fn(|_request: Request<Body>| {
        ready(Ok::<_, Infallible>(response(StatusCode::NO_CONTENT)))
    });
    let limiter = StubLimiter::returning(allowed(8, 7));
    let calls = Arc::clone(&limiter.calls);
    let key_layer = RateLimitLayer::new(
        limiter,
        policy(),
        |_request: &Request<Body>, _policy: &FixedWindowPolicy| {
            Err::<SubjectKey, _>("missing trusted client address")
        },
        |rejection| match rejection {
            RateLimitRejection::Key("missing trusted client address") => {
                response(StatusCode::BAD_REQUEST)
            }
            RateLimitRejection::Key(other) => panic!("unexpected key error {other:?}"),
            RateLimitRejection::Denied(_) | RateLimitRejection::Backend(StubError) => {
                panic!("a key failure must not reach the limiter")
            }
        },
    );

    let key_response = key_layer
        .layer(unused_inner)
        .oneshot(Request::new(Body::empty()))
        .await
        .unwrap();
    assert_eq!(key_response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(calls.load(Ordering::Relaxed), 0);

    let backend_layer = RateLimitLayer::new(
        StubLimiter::failing(),
        policy(),
        |_request: &Request<Body>, _policy: &FixedWindowPolicy| Ok::<_, Infallible>(subject(4)),
        |rejection| match rejection {
            RateLimitRejection::Key(never) => match never {},
            RateLimitRejection::Denied(_) => panic!("a backend failure is not a denial"),
            RateLimitRejection::Backend(StubError) => response(StatusCode::SERVICE_UNAVAILABLE),
        },
    );
    let backend_response = backend_layer
        .layer(unused_inner)
        .oneshot(Request::new(Body::empty()))
        .await
        .unwrap();
    assert_eq!(backend_response.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn forwarding_headers_have_no_effect_unless_the_extractor_uses_them() {
    let fixed_subject = subject(5);
    let limiter = StubLimiter::returning(allowed(8, 7));
    let observed_subjects = Arc::clone(&limiter.subjects);
    let ignoring_layer = RateLimitLayer::new(
        limiter,
        policy(),
        move |_request: &Request<Body>, _policy: &FixedWindowPolicy| {
            Ok::<_, Infallible>(fixed_subject)
        },
        |_rejection| panic!("unexpected rejection"),
    );
    let inner = service_fn(|_request: Request<Body>| {
        ready(Ok::<_, Infallible>(response(StatusCode::NO_CONTENT)))
    });

    for forwarded in ["198.51.100.1", "203.0.113.9"] {
        let mut request = Request::new(Body::empty());
        request
            .headers_mut()
            .insert("x-forwarded-for", HeaderValue::from_static(forwarded));
        ignoring_layer
            .clone()
            .layer(inner)
            .oneshot(request)
            .await
            .unwrap();
    }
    assert_eq!(
        observed_subjects.lock().unwrap().as_slice(),
        &[fixed_subject, fixed_subject]
    );

    let header_subject = subject(6);
    let limiter = StubLimiter::returning(allowed(8, 7));
    let observed_subjects = Arc::clone(&limiter.subjects);
    let using_layer = RateLimitLayer::new(
        limiter,
        policy(),
        move |request: &Request<Body>, _policy: &FixedWindowPolicy| {
            assert_eq!(request.headers()["x-forwarded-for"], "192.0.2.8");
            Ok::<_, Infallible>(header_subject)
        },
        |_rejection| panic!("unexpected rejection"),
    );
    let mut request = Request::new(Body::empty());
    request
        .headers_mut()
        .insert("x-forwarded-for", HeaderValue::from_static("192.0.2.8"));

    using_layer.layer(inner).oneshot(request).await.unwrap();
    assert_eq!(
        observed_subjects.lock().unwrap().as_slice(),
        &[header_subject]
    );
}

#[derive(Clone)]
struct ReadinessService {
    ready: Arc<AtomicBool>,
    ready_polls: Arc<AtomicUsize>,
    calls: Arc<AtomicUsize>,
}

impl Service<Request<Body>> for ReadinessService {
    type Response = Response;
    type Error = Infallible;
    type Future = Ready<Result<Response, Infallible>>;

    fn poll_ready(&mut self, _context: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.ready_polls.fetch_add(1, Ordering::Relaxed);
        if self.ready.load(Ordering::Relaxed) {
            Poll::Ready(Ok(()))
        } else {
            Poll::Pending
        }
    }

    fn call(&mut self, _request: Request<Body>) -> Self::Future {
        self.calls.fetch_add(1, Ordering::Relaxed);
        ready(Ok(response(StatusCode::NO_CONTENT)))
    }
}

#[tokio::test]
async fn readiness_is_forwarded_and_the_response_future_is_send() {
    let ready_flag = Arc::new(AtomicBool::new(false));
    let ready_polls = Arc::new(AtomicUsize::new(0));
    let calls = Arc::new(AtomicUsize::new(0));
    let inner = ReadinessService {
        ready: Arc::clone(&ready_flag),
        ready_polls: Arc::clone(&ready_polls),
        calls: Arc::clone(&calls),
    };
    let layer = RateLimitLayer::new(
        StubLimiter::returning(allowed(8, 7)),
        policy(),
        |_request: &Request<Body>, _policy: &FixedWindowPolicy| Ok::<_, Infallible>(subject(7)),
        |_rejection| panic!("unexpected rejection"),
    );
    let mut service = layer.layer(inner);
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);

    assert!(service.poll_ready(&mut context).is_pending());
    ready_flag.store(true, Ordering::Relaxed);
    assert!(matches!(
        service.poll_ready(&mut context),
        Poll::Ready(Ok(()))
    ));
    assert_eq!(ready_polls.load(Ordering::Relaxed), 2);

    let future = service.call(Request::new(Body::empty()));
    assert_send(&future);
    assert_eq!(future.await.unwrap().status(), StatusCode::NO_CONTENT);
    assert_eq!(calls.load(Ordering::Relaxed), 1);
}

struct ProbeBody {
    polled: Arc<AtomicBool>,
}

impl HttpBody for ProbeBody {
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(
        self: Pin<&mut Self>,
        _context: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        self.polled.store(true, Ordering::Relaxed);
        Poll::Ready(None)
    }

    fn size_hint(&self) -> SizeHint {
        SizeHint::default()
    }
}

#[tokio::test]
async fn rejected_request_body_is_never_polled() {
    let body_polled = Arc::new(AtomicBool::new(false));
    let inner_calls = Arc::new(AtomicUsize::new(0));
    let inner_calls_for_service = Arc::clone(&inner_calls);
    let inner = service_fn(move |_request: Request<ProbeBody>| {
        inner_calls_for_service.fetch_add(1, Ordering::Relaxed);
        ready(Ok::<_, Infallible>(response(StatusCode::NO_CONTENT)))
    });
    let layer = RateLimitLayer::new(
        StubLimiter::returning(Decision::denied(quota(8, Duration::from_secs(5)))),
        policy(),
        |_request: &Request<ProbeBody>, _policy: &FixedWindowPolicy| {
            Ok::<_, Infallible>(subject(8))
        },
        |_rejection: RateLimitRejection<Infallible, StubError>| {
            response(StatusCode::TOO_MANY_REQUESTS)
        },
    );
    let request = Request::new(ProbeBody {
        polled: Arc::clone(&body_polled),
    });

    let result = layer.layer(inner).oneshot(request).await.unwrap();

    assert_eq!(result.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(!body_polled.load(Ordering::Relaxed));
    assert_eq!(inner_calls.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn caller_can_choose_rejection_response_body() {
    let layer = RateLimitLayer::new(
        StubLimiter::failing(),
        policy(),
        |_request: &Request<Body>, _policy: &FixedWindowPolicy| Ok::<_, Infallible>(subject(9)),
        |_rejection| {
            Response::builder()
                .status(StatusCode::SERVICE_UNAVAILABLE)
                .body(Body::from("application-owned response"))
                .unwrap()
        },
    );
    let inner = service_fn(|_request: Request<Body>| {
        ready(Ok::<_, Infallible>(response(StatusCode::NO_CONTENT)))
    });

    let result = layer
        .layer(inner)
        .oneshot(Request::new(Body::empty()))
        .await
        .unwrap();
    let status = result.status();
    let body = to_bytes(result.into_body(), usize::MAX).await.unwrap();

    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body, "application-owned response");
}

#[test]
fn debug_output_is_useful_without_formatting_callback_state() {
    let secret = String::from("raw-identity-secret");
    let layer = RateLimitLayer::new(
        StubLimiter::returning(allowed(8, 7)),
        policy(),
        move |_request: &Request<Body>, _policy: &FixedWindowPolicy| {
            let _ = &secret;
            Ok::<_, Infallible>(subject(10))
        },
        |_rejection: RateLimitRejection<Infallible, StubError>| {
            response(StatusCode::TOO_MANY_REQUESTS)
        },
    );

    let debug = format!("{layer:?}");

    assert!(debug.contains("StubLimiter"));
    assert!(debug.contains("auth.login"));
    assert!(debug.contains("<redacted>"));
    assert!(!debug.contains("raw-identity-secret"));
}

#[test]
fn admissions_can_be_assembled_for_handler_tests() {
    let policy = policy();
    let admitted = Admitted::allowed(allowance(8, 7));
    let mut admissions = Admissions::new();
    assert!(admissions.is_empty());
    assert!(admissions.get(&policy).is_none());

    admissions.push(Admission::new(&policy, admitted));

    assert_eq!(
        admissions,
        Admissions::from(Admission::new(&policy, admitted))
    );
    assert_eq!(admissions.len(), 1);
    assert_eq!(
        admissions.get(&policy).map(Admission::decision),
        Some(admitted)
    );
    assert!(format!("{admissions:?}").contains("auth.login"));
}

fn assert_send<T: Send>(_value: &T) {}
