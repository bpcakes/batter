//! Protected authenticated HTTP admission with native atomic quotas.

use crate::quota::{Checks, ConsumptionError, Quota, RunResult, Snapshot};
use axum::{
    Json, Router,
    body::Body,
    extract::{ConnectInfo, FromRequestParts, Request, State},
    handler::Handler,
    http::{HeaderMap, StatusCode, header, request::Parts},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
};
use batter_axum::{
    RequestInterruptionResponder, RequestPolicy, operational_http_with_quota,
    quota_observation::{QuotaConsumption, QuotaRecorder, QuotaTerminalFacts},
    request_admission,
};
use batter_core::{
    operation::{OperationContext, OperationError},
    registration::RegistrationTarget,
};
use runlimit_core::{
    BatchDecisionView, Check, ConsumptionStatus, DenialView, Limiter, QuotaMode, RateLimitPolicy,
    SubjectKey,
};
use std::{
    convert::Infallible,
    future::Future,
    net::{IpAddr, SocketAddr},
    sync::Arc,
};
use tower::ServiceExt;

/// Supplied only by this boundary's native or explicit test transport.
#[derive(Clone, Copy)]
pub struct DirectPeer(IpAddr);
impl DirectPeer {
    /// The transport peer's address, never inferred from forwarding headers.
    pub fn ip(self) -> IpAddr {
        self.0
    }
}

/// Authentication can await native work under the request budget, but cannot read a body.
pub struct AuthInput {
    /// Owned request headers. Credential interpretation belongs to the application.
    pub headers: HeaderMap,
    /// Peer supplied by the serving boundary.
    pub peer: DirectPeer,
    /// Shared request budget and downward cancellation context.
    pub context: OperationContext,
}

/// Principal established by this boundary's authentication step.
///
/// The constructor is private, so an application layer cannot manufacture an
/// authenticated value from an unrelated extension of type `P`. Use this as a
/// handler extractor after a protected route has passed authentication.
///
/// ```
/// use batter_runlimit::http::Authenticated;
/// async fn handler(principal: Authenticated<&'static str>) -> String {
///     (*principal.principal()).to_owned()
/// }
/// ```
///
/// ```compile_fail
/// use batter_runlimit::http::Authenticated;
/// let forged = Authenticated("unverified");
/// ```
#[derive(Clone)]
pub struct Authenticated<P>(P);

impl<P> Authenticated<P> {
    /// Borrow the authenticated application principal.
    pub fn principal(&self) -> &P {
        &self.0
    }
}

impl<P, S> FromRequestParts<S> for Authenticated<P>
where
    P: Clone + Send + Sync + 'static,
    S: Send + Sync,
{
    type Rejection = StatusCode;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<Self>()
            .cloned()
            .ok_or(StatusCode::INTERNAL_SERVER_ERROR)
    }
}

/// Rejected public probe path configuration.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum PublicProbePathError {
    /// Axum route paths must start with a slash.
    #[error("public probe path must start with '/'")]
    NotAbsolute,
    /// A public probe must name one literal path, not a route pattern.
    #[error("public probe path must be literal, without capture or wildcard syntax")]
    Pattern,
}

/// Explicit literal GET probe paths that bypass the protected request gates.
///
/// This builder cannot hold a caller-supplied `Router`, root fallback, nested
/// fallback, custom method fallback, capture, or catch-all route. `GET` also
/// serves `HEAD` through Axum.
/// Other methods on a registered path receive Axum's ordinary 405 response;
/// they do not invoke an application fallback handler.
///
/// ```
/// use batter_runlimit::http::PublicProbes;
/// let probes = PublicProbes::new().get("/live", || async { "live" }).unwrap();
/// # let _ = probes;
/// ```
///
/// ```compile_fail
/// use batter_runlimit::http::PublicProbes;
/// fn accept(_: PublicProbes) {}
/// accept(axum::Router::new());
/// ```
///
/// ```compile_fail
/// use batter_runlimit::http::PublicProbes;
/// let _ = PublicProbes::new().method_not_allowed_fallback(|| async { "public" });
/// ```
#[must_use]
#[derive(Default)]
pub struct PublicProbes {
    router: Router,
}

impl PublicProbes {
    /// Start an empty explicit probe set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add one literal public GET path; Axum also serves HEAD for this path.
    /// The handler runs without request admission, authentication, or quota.
    /// Capture and catch-all patterns cannot make other paths public.
    ///
    /// # Errors
    ///
    /// Returns [`PublicProbePathError`] when `path` is not absolute or contains
    /// Axum capture or wildcard syntax. Axum still validates other route syntax.
    pub fn get<H, T>(mut self, path: &str, handler: H) -> Result<Self, PublicProbePathError>
    where
        H: Handler<T, ()>,
        T: 'static,
    {
        if !path.starts_with('/') {
            return Err(PublicProbePathError::NotAbsolute);
        }
        if path
            .bytes()
            .any(|byte| matches!(byte, b'{' | b'}' | b'*' | b':'))
        {
            return Err(PublicProbePathError::Pattern);
        }
        self.router = self.router.route(path, get(handler));
        Ok(self)
    }
}

/// Invalid static policy configuration for a protected HTTP quota boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum HttpQuotaConfigError {
    /// A protected HTTP boundary needs at least one native policy.
    #[error("at least one quota policy is required")]
    EmptyPolicies,
    /// Native atomic batches cannot combine enforced and shadow policies.
    #[error(
        "quota policy at index {index} uses {actual:?}, unlike the first policy's {first:?} mode"
    )]
    MixedQuotaModes {
        /// Quota mode of the first policy.
        first: QuotaMode,
        /// Index of the first policy with a different quota mode.
        index: usize,
        /// Quota mode at `index`.
        actual: QuotaMode,
    },
}

/// Authenticates a principal, derives native subjects, and checks one atomic batch.
///
/// This is not a pre-authentication throttle. The constructor enforces a
/// uniform quota mode; Runlimit validates the remaining native batch rules.
/// Authentication and
/// subject selection must not spawn unowned work; dropping their futures cannot
/// prove detached tasks or remote effects have stopped.
pub struct HttpQuota<L, A, K>
where
    L: Limiter,
{
    quota: Quota<L>,
    policies: Vec<L::Policy>,
    authenticate: A,
    subject: K,
    public_probes: Option<PublicProbes>,
}

impl<L, A, K> HttpQuota<L, A, K>
where
    L: Limiter + 'static,
    L::Error: ConsumptionError,
{
    /// Builds a boundary with nonempty policies of one native quota mode.
    /// Subject-dependent batch validation remains with Runlimit; native errors
    /// are retained in response extensions as `Arc<E>`.
    /// Authentication and subject-selector signatures are checked here.
    ///
    /// A wrong authentication input fails at construction:
    ///
    /// ```compile_fail
    /// use batter_runlimit::{ConsumptionError, Quota};
    /// use batter_runlimit::http::{AuthInput, DirectPeer, HttpQuota};
    /// use runlimit_core::{Limiter, SubjectKey};
    ///
    /// fn wrong_auth<L: Limiter + 'static>(quota: Quota<L>, policies: Vec<L::Policy>)
    /// where
    ///     L::Error: ConsumptionError,
    /// {
    ///     let _ = HttpQuota::new(
    ///         quota, policies,
    ///         |_wrong: u8| async { Ok::<&'static str, std::io::Error>("owner") },
    ///         |_principal: &&'static str, _peer: DirectPeer, _policy: &L::Policy| -> SubjectKey { todo!() },
    ///     );
    /// }
    /// ```
    ///
    /// A selector for another principal type also fails at construction:
    ///
    /// ```compile_fail
    /// use batter_runlimit::{ConsumptionError, Quota};
    /// use batter_runlimit::http::{AuthInput, DirectPeer, HttpQuota};
    /// use runlimit_core::{Limiter, SubjectKey};
    ///
    /// fn wrong_selector<L: Limiter + 'static>(quota: Quota<L>, policies: Vec<L::Policy>)
    /// where
    ///     L::Error: ConsumptionError,
    /// {
    ///     let _ = HttpQuota::new(
    ///         quota, policies,
    ///         |_input: AuthInput| async { Ok::<&'static str, std::io::Error>("owner") },
    ///         |_wrong: &u64, _peer: DirectPeer, _policy: &L::Policy| -> SubjectKey { todo!() },
    ///     );
    /// }
    /// ```
    pub fn new<P, AE, AF>(
        quota: Quota<L>,
        policies: Vec<L::Policy>,
        authenticate: A,
        subject: K,
    ) -> Result<Self, HttpQuotaConfigError>
    where
        P: Clone + Send + Sync + 'static,
        AE: std::error::Error + Send + Sync + 'static,
        A: Fn(AuthInput) -> AF + Send + Sync + 'static,
        AF: Future<Output = Result<P, AE>> + Send + 'static,
        K: Fn(&P, DirectPeer, &L::Policy) -> SubjectKey + Send + Sync + 'static,
    {
        let Some(first) = policies.first() else {
            return Err(HttpQuotaConfigError::EmptyPolicies);
        };
        let first_mode = first.quota_mode();
        for (index, policy) in policies.iter().enumerate().skip(1) {
            let actual = policy.quota_mode();
            if actual != first_mode {
                return Err(HttpQuotaConfigError::MixedQuotaModes {
                    first: first_mode,
                    index,
                    actual,
                });
            }
        }
        Ok(Self {
            quota,
            policies,
            authenticate,
            subject,
            public_probes: None,
        })
    }

    /// Explicitly add public GET probe paths (and Axum's corresponding HEAD).
    ///
    /// These handlers bypass lifecycle/deadline request admission,
    /// authentication, and quota. They receive no protected `OperationContext`.
    /// Registered probe paths and their default method responses still receive
    /// the root HTTP observation and correlation ID. The default is no public
    /// probes; the builder accepts only literal paths and cannot install public
    /// fallback handlers. If a public probe and a protected route both
    /// register GET at the same path, [`Self::prepare`] panics during route
    /// assembly because Axum rejects overlapping method routes.
    pub fn with_public_probes(mut self, probes: PublicProbes) -> Self {
        self.public_probes = Some(probes);
        self
    }

    /// All routes are assembled before the sole HTTP observer is installed.
    ///
    /// Lifecycle/deadline admission precedes authentication; authentication and quota
    /// checking precede body extraction. All endpoints in `routes`, including
    /// custom root and nested fallbacks, receive those gates. Without a custom
    /// protected fallback, unmatched requests receive the default public 404.
    /// Routes supplied through
    /// [`with_public_probes`](Self::with_public_probes) bypass those gates and
    /// receive no protected `OperationContext`; they share the root observer.
    /// [`Authenticated<P>`] is installed only after successful authentication;
    /// handlers must extract it rather than a raw `Extension<P>`.
    /// A public probe and protected GET route at the same path cause Axum's
    /// router merge to panic here, before the service can be registered.
    /// Use distinct paths for public and protected GET handlers.
    /// Response-body streaming is outside the operation and observation lifetime.
    pub fn prepare<P, AE, AF>(mut self, policy: RequestPolicy, routes: Router) -> PreparedHttp
    where
        P: Clone + Send + Sync + 'static,
        AE: std::error::Error + Send + Sync + 'static,
        A: Fn(AuthInput) -> AF + Send + Sync + 'static,
        AF: Future<Output = Result<P, AE>> + Send + 'static,
        K: Fn(&P, DirectPeer, &L::Policy) -> SubjectKey + Send + Sync + 'static,
    {
        let probes = self
            .public_probes
            .take()
            .map(|probes| {
                probes
                    .router
                    .layer(middleware::from_fn(discard_probe_writer))
            })
            .unwrap_or_default();
        // Axum route_layer excludes nested fallbacks; protect the whole Router.
        let business = routes
            .layer(middleware::from_fn_with_state(
                Arc::new(self),
                boundary::<L, A, K, P, AE, AF>,
            ))
            .layer(middleware::from_fn_with_state(policy, request_admission));
        PreparedHttp {
            router: business
                .merge(probes)
                .layer(middleware::from_fn(operational_http_with_quota)),
        }
    }
}

/// Cannot expose a Router that callers serve without required metadata.
///
/// ```compile_fail
/// fn bypass(prepared: batter_runlimit::http::PreparedHttp) -> axum::Router {
///     prepared
/// }
/// ```
pub struct PreparedHttp {
    router: Router,
}
impl PreparedHttp {
    /// Registers native connect-info serving as critical component `quota.http`.
    /// A target can contain only one component with that name. The target owns
    /// readiness and shutdown; the listener is already acquired by the caller.
    pub fn register_in<T: RegistrationTarget + ?Sized>(
        self,
        target: &mut T,
        listener: tokio::net::TcpListener,
    ) -> Result<(), batter_core::RegistrationError> {
        batter_axum::register_http_with_connect_info_in(target, "quota.http", listener, self.router)
    }
    /// Consumes the prepared service into an explicit synthetic test transport.
    pub fn in_process(self) -> TestClient {
        TestClient {
            router: self.router,
        }
    }
}

/// Explicit synthetic transport; does not implement Service or expose its Router.
///
/// ```compile_fail
/// async fn cannot_serve(listener: tokio::net::TcpListener, client: batter_runlimit::http::TestClient) {
///     axum::serve(listener, client).await.unwrap();
/// }
/// ```
#[derive(Clone)]
pub struct TestClient {
    router: Router,
}

async fn discard_probe_writer(mut request: Request, next: Next) -> Response {
    // The root observer still reports NotChecked, but application probe code
    // cannot claim the writer and fabricate a native quota outcome.
    let _ = QuotaRecorder::take(&mut request);
    next.run(request).await
}

#[derive(Clone, Copy)]
enum BoundaryRejection {
    MissingContext,
    MissingPeer,
    MissingObserver,
    AuthenticationRequired,
    QuotaExhausted,
    QuotaStorageCapacity,
    QuotaBackendFailed,
}

impl BoundaryRejection {
    fn status(self) -> StatusCode {
        match self {
            Self::MissingContext | Self::MissingPeer | Self::MissingObserver => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
            Self::AuthenticationRequired => StatusCode::UNAUTHORIZED,
            Self::QuotaExhausted => StatusCode::TOO_MANY_REQUESTS,
            Self::QuotaStorageCapacity | Self::QuotaBackendFailed => {
                StatusCode::SERVICE_UNAVAILABLE
            }
        }
    }

    fn code(self) -> &'static str {
        match self {
            Self::MissingContext => "missing_context",
            Self::MissingPeer => "missing_peer",
            Self::MissingObserver => "missing_observer",
            Self::AuthenticationRequired => "authentication_required",
            Self::QuotaExhausted => "quota_exhausted",
            Self::QuotaStorageCapacity => "quota_storage_capacity",
            Self::QuotaBackendFailed => "quota_backend_failed",
        }
    }
}

impl TestClient {
    /// Dispatches with the supplied synthetic peer, replacing any request peer.
    pub async fn request(&self, mut request: Request<Body>, peer: SocketAddr) -> Response {
        request.extensions_mut().insert(ConnectInfo(peer));
        self.router.clone().oneshot(request).await.unwrap()
    }
}

async fn boundary<L, A, K, P, AE, AF>(
    State(boundary): State<Arc<HttpQuota<L, A, K>>>,
    mut request: Request,
    next: Next,
) -> Response
where
    L: Limiter + 'static,
    L::Error: ConsumptionError,
    P: Clone + Send + Sync + 'static,
    AE: std::error::Error + Send + Sync + 'static,
    A: Fn(AuthInput) -> AF + Send + Sync + 'static,
    AF: Future<Output = Result<P, AE>> + Send,
    K: Fn(&P, DirectPeer, &L::Policy) -> SubjectKey + Send + Sync + 'static,
{
    // Never accept an already-installed principal as authentication evidence.
    request.extensions_mut().remove::<Authenticated<P>>();
    let Some(context) = request.extensions().get::<OperationContext>().cloned() else {
        return rejection(BoundaryRejection::MissingContext);
    };
    let interruption_responder = request
        .extensions()
        .get::<RequestInterruptionResponder>()
        .cloned()
        .expect("request admission installs an interruption responder");
    let Some(peer) = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|peer| DirectPeer(peer.0.ip()))
    else {
        return rejection(BoundaryRejection::MissingPeer);
    };
    let Some(record) = QuotaRecorder::take(&mut request) else {
        return rejection(BoundaryRejection::MissingObserver);
    };
    let mut unstarted = Some(record);
    let mut started = None;
    let principal = match (boundary.authenticate)(AuthInput {
        headers: request.headers().clone(),
        peer,
        context: context.clone(),
    })
    .await
    {
        Ok(principal) => principal,
        Err(error) => {
            let mut response = rejection(BoundaryRejection::AuthenticationRequired);
            response.extensions_mut().insert(Arc::new(error));
            return response;
        }
    };
    let checks = boundary
        .policies
        .iter()
        .map(|policy| Check::new(policy, (boundary.subject)(&principal, peer, policy)))
        .collect::<Vec<_>>();
    request.extensions_mut().insert(Authenticated(principal));
    match boundary
        .quota
        .run_recorded(
            &context,
            Checks::new(&checks).unwrap(),
            |snapshot| match snapshot {
                Snapshot::Started => {
                    started = Some(unstarted.take().expect("one quota check").start());
                }
                Snapshot::Decided(_) | Snapshot::Failed(_) => {
                    started
                        .take()
                        .expect("started quota check")
                        .finish(project_terminal(snapshot));
                }
            },
            |_| async move { Ok::<_, Infallible>(next.run(request).await) },
        )
        .await
    {
        RunResult::Admitted {
            work: Ok(response), ..
        } => response,
        RunResult::Admitted {
            work: Err(OperationError::Failed(never)),
            ..
        } => match never {},
        RunResult::Admitted {
            work: Err(OperationError::Interrupted(reason)),
            ..
        }
        | RunResult::Interrupted { reason, .. } => interruption_responder.render(reason),
        RunResult::Backend { error, .. } => {
            let mut response = rejection(BoundaryRejection::QuotaBackendFailed);
            response.extensions_mut().insert(Arc::new(error));
            response
        }
        RunResult::Rejected { denial, .. } => {
            let (kind, retry_after) = match denial.view() {
                DenialView::QuotaExceeded(details) => (
                    BoundaryRejection::QuotaExhausted,
                    Some(details.retry_after()),
                ),
                DenialView::StorageCapacity { retry_after } => {
                    (BoundaryRejection::QuotaStorageCapacity, retry_after)
                }
            };
            let mut response = rejection(kind);
            if let Some(seconds) = retry_after.map(|delay| delay.seconds()) {
                response
                    .headers_mut()
                    .insert(header::RETRY_AFTER, seconds.to_string().parse().unwrap());
            }
            response
        }
    }
}

fn project_terminal(snapshot: &Snapshot) -> QuotaTerminalFacts {
    match snapshot {
        Snapshot::Failed(consumption) => QuotaTerminalFacts::BackendFailed(match consumption {
            ConsumptionStatus::Consumed => QuotaConsumption::Consumed,
            ConsumptionStatus::NotConsumed => QuotaConsumption::NotConsumed,
            ConsumptionStatus::PossiblyConsumed => QuotaConsumption::Unknown,
        }),
        Snapshot::Decided(decision) => match decision.view() {
            BatchDecisionView::Allowed { .. } => QuotaTerminalFacts::Allowed,
            BatchDecisionView::ShadowDenied { .. } => QuotaTerminalFacts::ShadowDenied,
            BatchDecisionView::Denied { denial, .. } => match denial {
                DenialView::QuotaExceeded(_) => QuotaTerminalFacts::QuotaDenied,
                DenialView::StorageCapacity { .. } => QuotaTerminalFacts::StorageCapacity,
            },
        },
        Snapshot::Started => unreachable!("only completed quota results"),
    }
}

fn rejection(kind: BoundaryRejection) -> Response {
    (
        kind.status(),
        [(header::CACHE_CONTROL, "no-store")],
        Json(serde_json::json!({ "code": kind.code() })),
    )
        .into_response()
}

#[cfg(test)]
mod boundary_rejection_tests {
    use super::*;

    #[test]
    fn fixed_status_and_code_values_are_pinned() {
        for (kind, status, code) in [
            (
                BoundaryRejection::MissingContext,
                StatusCode::INTERNAL_SERVER_ERROR,
                "missing_context",
            ),
            (
                BoundaryRejection::MissingPeer,
                StatusCode::INTERNAL_SERVER_ERROR,
                "missing_peer",
            ),
            (
                BoundaryRejection::MissingObserver,
                StatusCode::INTERNAL_SERVER_ERROR,
                "missing_observer",
            ),
            (
                BoundaryRejection::AuthenticationRequired,
                StatusCode::UNAUTHORIZED,
                "authentication_required",
            ),
            (
                BoundaryRejection::QuotaExhausted,
                StatusCode::TOO_MANY_REQUESTS,
                "quota_exhausted",
            ),
            (
                BoundaryRejection::QuotaStorageCapacity,
                StatusCode::SERVICE_UNAVAILABLE,
                "quota_storage_capacity",
            ),
            (
                BoundaryRejection::QuotaBackendFailed,
                StatusCode::SERVICE_UNAVAILABLE,
                "quota_backend_failed",
            ),
        ] {
            assert_eq!(kind.status(), status);
            assert_eq!(kind.code(), code);
        }
    }
}
