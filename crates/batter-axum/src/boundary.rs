//! Library-owned HTTP composition with a fixed layer order.

use crate::{
    ReadinessPolicy, RequestPolicy, dependency_readiness, liveness, operational_http,
    request_admission, serving,
};
use axum::{
    Router,
    body::Body,
    extract::Request,
    handler::Handler,
    http::{Method, StatusCode},
    middleware,
    response::{IntoResponse, Response},
    routing::{MethodRouter, Route, any, get},
};
use batter_core::{RegistrationError, registration::RegistrationTarget};
use std::{convert::Infallible, error::Error, fmt};
use tokio::net::TcpListener;
use tower::{Layer, Service, ServiceExt};

/// A validated, literal route for a process probe.
///
/// Probe paths deliberately accept only absolute ASCII paths made from
/// unreserved URI characters. Axum capture, wildcard and legacy pattern syntax
/// cannot be represented, so a probe cannot become a public fallback for
/// guarded application routes.
///
/// ```
/// use batter_axum::ProbePath;
///
/// let path = ProbePath::new("/health/live")?;
/// assert_eq!(path.as_str(), "/health/live");
/// # Ok::<(), batter_axum::ProbePathError>(())
/// ```
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[must_use = "retain the validated probe path"]
pub struct ProbePath(&'static str);

impl ProbePath {
    /// Validate one absolute, literal probe path.
    ///
    /// Each non-empty segment may contain only ASCII letters, digits, `-`,
    /// `.`, `_`, or `~`. Empty, dot, capture, wildcard, query, fragment and
    /// percent-encoded segments are rejected. `/` is accepted as the root
    /// literal. The input is retained only after validation and is never
    /// included in the sanitized error.
    pub fn new(path: &'static str) -> Result<Self, ProbePathError> {
        validate_probe_path(path)?;
        Ok(Self(path))
    }

    /// Return the exact validated route handed to Axum.
    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

fn validate_probe_path(path: &str) -> Result<(), ProbePathError> {
    if !path.starts_with('/') {
        return Err(ProbePathError::NotAbsolute);
    }
    if path == "/" {
        return Ok(());
    }
    if path.len() > 256 {
        return Err(ProbePathError::TooLong);
    }
    for segment in path[1..].split('/') {
        if segment.is_empty() {
            return Err(ProbePathError::EmptySegment);
        }
        if segment == "." || segment == ".." {
            return Err(ProbePathError::DotSegment);
        }
        if segment
            .bytes()
            .any(|byte| !byte.is_ascii_alphanumeric() && !b"-._~".contains(&byte))
        {
            return Err(ProbePathError::NonLiteralSegment);
        }
    }
    Ok(())
}

/// Sanitized failure to construct a literal [`ProbePath`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ProbePathError {
    /// The path is empty or does not begin with `/`.
    NotAbsolute,
    /// The path is longer than the boundary's fixed configuration limit.
    TooLong,
    /// The path contains a repeated separator or trailing slash.
    EmptySegment,
    /// The path contains `.` or `..` as a complete segment.
    DotSegment,
    /// A segment contains route syntax or a non-unreserved character.
    NonLiteralSegment,
}

impl fmt::Display for ProbePathError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NotAbsolute => "probe path must be absolute",
            Self::TooLong => "probe path is too long",
            Self::EmptySegment => "probe path contains an empty segment",
            Self::DotSegment => "probe path contains a dot segment",
            Self::NonLiteralSegment => "probe path contains non-literal route syntax",
        })
    }
}

impl Error for ProbePathError {}

/// Sanitized failure to register a probe in an [`HttpBoundary`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ProbeRegistrationError {
    /// The same literal path was already assigned to another probe.
    DuplicatePath,
}

impl fmt::Display for ProbeRegistrationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::DuplicatePath => "probe path is already registered",
        })
    }
}

impl Error for ProbeRegistrationError {}

/// Sanitized failure to combine probes with guarded application routes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum BoundaryAssemblyError {
    /// A guarded route can match a path reserved for a public probe.
    GuardedProbePath,
}

impl fmt::Display for BoundaryAssemblyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::GuardedProbePath => "guarded route conflicts with a probe path",
        })
    }
}

impl Error for BoundaryAssemblyError {}

/// Guarded application routes whose identities remain visible to [`HttpBoundary`].
///
/// This is the canonical application-router builder. It mirrors the Axum
/// operations that preserve route identity while retaining every declared path
/// separately from the native router. That retained inventory lets boundary
/// assembly reject probe collisions without executing application services.
/// Arbitrary [`Router::nest_service`] input is deliberately absent because an
/// opaque service exposes no route inventory; use [`Self::nest`] with another
/// `GuardedRouter` instead.
///
/// ```
/// use axum::{http::StatusCode, routing::get};
/// use batter_axum::GuardedRouter;
///
/// let nested: GuardedRouter =
///     GuardedRouter::new().route("/work", get(|| async { "ok" }));
/// let guarded = GuardedRouter::new()
///     .nest("/api", nested)
///     .fallback(|| async { StatusCode::NOT_FOUND });
/// # let _ = guarded;
/// ```
///
/// Opaque nested services cannot enter the protected path:
///
/// ```compile_fail,E0599
/// use axum::Router;
/// use batter_axum::GuardedRouter;
///
/// let guarded = GuardedRouter::new().nest_service("/api", Router::new());
/// ```
#[must_use = "pass the guarded routes to HttpBoundary::assemble"]
pub struct GuardedRouter<S = ()> {
    router: Router<S>,
    route_patterns: Vec<String>,
}

impl<S> GuardedRouter<S>
where
    S: Clone + Send + Sync + 'static,
{
    /// Start an empty guarded application router.
    pub fn new() -> Self {
        Self {
            router: Router::new(),
            route_patterns: Vec::new(),
        }
    }

    /// Add an Axum method router and retain its route pattern for assembly validation.
    pub fn route(mut self, path: &str, method_router: MethodRouter<S>) -> Self {
        self.router = self.router.route(path, method_router);
        if !self.route_patterns.iter().any(|known| known == path) {
            self.route_patterns.push(path.to_owned());
        }
        self
    }

    /// Add an infallible service at one guarded route and retain its pattern.
    pub fn route_service<T>(mut self, path: &str, service: T) -> Self
    where
        T: Service<Request, Error = Infallible> + Clone + Send + Sync + 'static,
        T::Response: IntoResponse,
        T::Future: Send + 'static,
    {
        self.router = self.router.route_service(path, service);
        if !self.route_patterns.iter().any(|known| known == path) {
            self.route_patterns.push(path.to_owned());
        }
        self
    }

    /// Nest guarded routes while retaining their fully qualified route patterns.
    pub fn nest(mut self, path: &str, nested: GuardedRouter<S>) -> Self {
        self.router = self.router.nest(path, nested.router);
        for nested_path in nested.route_patterns {
            let full_path = if path.ends_with('/') {
                format!("{path}{}", nested_path.trim_start_matches('/'))
            } else if nested_path == "/" {
                path.to_owned()
            } else {
                format!("{path}{nested_path}")
            };
            if !self.route_patterns.contains(&full_path) {
                self.route_patterns.push(full_path);
            }
        }
        self
    }

    /// Merge another guarded router and retain both route inventories.
    pub fn merge(mut self, other: GuardedRouter<S>) -> Self {
        self.router = self.router.merge(other.router);
        for path in other.route_patterns {
            if !self.route_patterns.contains(&path) {
                self.route_patterns.push(path);
            }
        }
        self
    }

    /// Set the guarded fallback without treating it as an explicit route identity.
    pub fn fallback<H, T>(mut self, handler: H) -> Self
    where
        H: Handler<T, S>,
        T: 'static,
    {
        self.router = self.router.fallback(handler);
        self
    }

    /// Set an infallible guarded fallback service.
    pub fn fallback_service<T>(mut self, service: T) -> Self
    where
        T: Service<Request, Error = Infallible> + Clone + Send + Sync + 'static,
        T::Response: IntoResponse,
        T::Future: Send + 'static,
    {
        self.router = self.router.fallback_service(service);
        self
    }

    /// Apply a native Axum layer to every current guarded route and fallback.
    pub fn layer<L>(mut self, layer: L) -> Self
    where
        L: Layer<Route> + Clone + Send + Sync + 'static,
        L::Service: Service<Request> + Clone + Send + Sync + 'static,
        <L::Service as Service<Request>>::Response: IntoResponse + 'static,
        <L::Service as Service<Request>>::Error: Into<Infallible> + 'static,
        <L::Service as Service<Request>>::Future: Send + 'static,
    {
        self.router = self.router.layer(layer);
        self
    }

    /// Apply a native Axum layer to every current explicit guarded route.
    pub fn route_layer<L>(mut self, layer: L) -> Self
    where
        L: Layer<Route> + Clone + Send + Sync + 'static,
        L::Service: Service<Request> + Clone + Send + Sync + 'static,
        <L::Service as Service<Request>>::Response: IntoResponse + 'static,
        <L::Service as Service<Request>>::Error: Into<Infallible> + 'static,
        <L::Service as Service<Request>>::Future: Send + 'static,
    {
        self.router = self.router.route_layer(layer);
        self
    }

    /// Supply native Axum state while retaining the guarded route inventory.
    pub fn with_state<S2>(self, state: S) -> GuardedRouter<S2> {
        GuardedRouter {
            router: self.router.with_state(state),
            route_patterns: self.route_patterns,
        }
    }
}

impl<S> Default for GuardedRouter<S>
where
    S: Clone + Send + Sync + 'static,
{
    fn default() -> Self {
        Self::new()
    }
}

/// Library-owned HTTP composition.
///
/// The assembled router has one fixed shape: server correlation and the single
/// HTTP observer outermost, probes mounted outside admission, and lifecycle
/// admission with the response-construction deadline around every guarded
/// route, including nested fallbacks. The caller supplies only the policy, the
/// probe paths and the guarded application; the order cannot be changed and no
/// probe can end up inside the admission gate. This is the canonical path.
/// [`crate::observe_http`], [`crate::request_admission`],
/// [`crate::request_scope`] and [`crate::operational_http`] remain available for
/// compositions the boundary cannot express; each documents the ordering it
/// then leaves with the caller.
///
/// ```
/// use axum::{Extension, Router, routing::get};
/// use batter_core::{
///     health::{HealthMonitor, HealthPolicy},
///     lifecycle::ShutdownHandle,
///     operation::OperationContext,
/// };
/// use batter_axum::{
///     HttpBoundary, ProbePath, ReadinessPolicy, RequestPolicy, ResponseConstructionBudget,
/// };
/// use std::{convert::Infallible, time::Duration};
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let (control, approval) = ShutdownHandle::new_with_readiness_approval();
/// approval.approve();
/// let second = Duration::from_secs(1);
/// let policy = HealthPolicy::new(second, second, second * 3, second)?;
/// let monitor = HealthMonitor::new(policy, || async { Ok::<_, Infallible>(()) });
/// let guarded = batter_axum::GuardedRouter::new().route(
///     "/work",
///     get(|Extension(context): Extension<OperationContext>| async move {
///         context.check().expect("admitted context");
///         "ok"
///     }),
/// );
/// let budget = ResponseConstructionBudget::new(second)?;
/// let assembled = HttpBoundary::new(RequestPolicy::new(control.operation_admission(), budget))
///     .with_liveness(ProbePath::new("/live")?)?
///     .with_readiness(
///         ProbePath::new("/ready")?,
///         ReadinessPolicy::new(control.status(), monitor.reader()),
///     )?
///     .assemble(guarded)
///     .await?;
/// // Inside protected startup: assembled.register_in(scope, "http", listener)?;
/// # let _ = assembled.into_router();
/// # drop(monitor);
/// # Ok(()) }
/// ```
#[must_use = "assemble the boundary into a router and register it"]
pub struct HttpBoundary {
    policy: RequestPolicy,
    probes: Router,
    probe_paths: Vec<ProbePath>,
}

impl HttpBoundary {
    /// Start a boundary with the admission policy for guarded routes and no probes.
    pub fn new(policy: RequestPolicy) -> Self {
        Self {
            policy,
            probes: Router::new(),
            probe_paths: Vec::new(),
        }
    }

    /// Mount the process liveness probe at a validated literal path outside admission.
    ///
    /// Raw strings cannot cross this boundary:
    ///
    /// ```compile_fail,E0308
    /// # use batter_axum::{HttpBoundary, RequestPolicy};
    /// # fn invalid(boundary: HttpBoundary) {
    /// boundary.with_liveness("/{*path}");
    /// # }
    /// ```
    pub fn with_liveness(mut self, path: ProbePath) -> Result<Self, ProbeRegistrationError> {
        self.reserve_probe(path)?;
        self.probes = self.probes.route(path.as_str(), get(liveness));
        Ok(self)
    }

    /// Mount lifecycle-plus-dependency readiness at a validated literal path.
    pub fn with_readiness<E: Send + Sync + 'static>(
        mut self,
        path: ProbePath,
        policy: ReadinessPolicy<E>,
    ) -> Result<Self, ProbeRegistrationError> {
        self.reserve_probe(path)?;
        self.probes = self.probes.route(
            path.as_str(),
            get(dependency_readiness::<E>).with_state(policy),
        );
        Ok(self)
    }

    fn reserve_probe(&mut self, path: ProbePath) -> Result<(), ProbeRegistrationError> {
        if self.probe_paths.contains(&path) {
            return Err(ProbeRegistrationError::DuplicatePath);
        }
        self.probe_paths.push(path);
        Ok(())
    }

    /// Validate that guarded routes do not occupy probe paths, apply admission
    /// to every guarded route, merge the probes outside it, and install
    /// correlation with the single HTTP observer outermost.
    ///
    /// Unmatched paths inside `guarded`, including nested fallbacks, receive
    /// the admission gate. Routes added to the result afterward would sit
    /// outside the boundary, so the result is not a bare [`Router`].
    /// Route validation polls only a library-owned inert inventory. It does not
    /// poll guarded handlers, fallbacks, or their middleware. A probe path
    /// reserves the complete route identity, so even a different guarded
    /// method at that path is rejected before Axum merge can panic.
    pub async fn assemble(
        self,
        guarded: GuardedRouter,
    ) -> Result<AssembledHttp, BoundaryAssemblyError> {
        let mut inspection = Router::new();
        for path in &guarded.route_patterns {
            inspection = inspection.route(path, any(report_guarded_route_match));
        }
        for path in &self.probe_paths {
            let request = Request::builder()
                .method(Method::OPTIONS)
                .uri(path.as_str())
                .body(Body::empty())
                .expect("validated probe path forms an HTTP request");
            let response = inspection
                .clone()
                .oneshot(request)
                .await
                .expect("Axum routers are infallible services");
            if response.extensions().get::<GuardedRouteMatch>().is_some() {
                return Err(BoundaryAssemblyError::GuardedProbePath);
            }
        }
        let guarded = guarded.router.layer(middleware::from_fn_with_state(
            self.policy,
            request_admission,
        ));
        Ok(AssembledHttp {
            // With two default fallbacks Axum retains the second router's;
            // a second custom fallback also supersedes a first default. Keep
            // the guarded router second so its layered fallback is retained.
            router: self
                .probes
                .merge(guarded)
                .layer(middleware::from_fn(operational_http)),
        })
    }
}

#[derive(Clone, Copy)]
struct GuardedRouteMatch;

async fn report_guarded_route_match() -> Response {
    let mut response = StatusCode::NO_CONTENT.into_response();
    response.extensions_mut().insert(GuardedRouteMatch);
    response
}

/// A router with the boundary applied.
///
/// It is served only through Batter's registration helpers, so no layer can be
/// added outside the observer by accident. A bare [`Router`] cannot be passed
/// where this is expected:
///
/// ```compile_fail,E0308
/// fn serve(assembled: batter_axum::AssembledHttp) -> axum::Router {
///     assembled
/// }
/// ```
#[must_use = "register the assembled boundary or take its router explicitly"]
pub struct AssembledHttp {
    router: Router,
}

impl AssembledHttp {
    /// Register the assembled server through constrained registration authority.
    /// See [`crate::register_http_in`] for the serving contract.
    pub fn register_in<T: RegistrationTarget + ?Sized>(
        self,
        target: &mut T,
        name: &'static str,
        listener: TcpListener,
    ) -> Result<(), RegistrationError> {
        serving::register_http_in(target, name, listener, self.router)
    }

    /// Register with the direct TCP peer available to handlers.
    /// See [`crate::register_http_with_connect_info_in`] for the contract.
    pub fn register_with_connect_info_in<T: RegistrationTarget + ?Sized>(
        self,
        target: &mut T,
        name: &'static str,
        listener: TcpListener,
    ) -> Result<(), RegistrationError> {
        serving::register_http_with_connect_info_in(target, name, listener, self.router)
    }

    /// Take the router for in-process tests with `tower::ServiceExt::oneshot`.
    ///
    /// Layers or routes added afterward sit outside the boundary; a nested
    /// observer added this way observes nothing because the outermost observer
    /// already owns the request.
    pub fn into_router(self) -> Router {
        self.router
    }
}

#[cfg(test)]
mod tests {
    use super::{
        BoundaryAssemblyError, GuardedRouter, HttpBoundary, ProbePath, ProbePathError,
        ProbeRegistrationError,
    };
    use crate::{ReadinessPolicy, RequestPolicy, ResponseConstructionBudget};
    use batter_core::{
        health::{HealthMonitor, HealthPolicy},
        lifecycle::ShutdownHandle,
    };
    use std::{convert::Infallible, time::Duration};

    #[test]
    fn probe_path_accepts_only_static_absolute_routes() {
        for valid in ["/", "/live", "/health/ready-v2", "/_internal/~probe"] {
            assert_eq!(ProbePath::new(valid).unwrap().as_str(), valid);
        }
        for (invalid, expected) in [
            ("", ProbePathError::NotAbsolute),
            ("live", ProbePathError::NotAbsolute),
            ("/live/", ProbePathError::EmptySegment),
            ("/health//live", ProbePathError::EmptySegment),
            ("/./live", ProbePathError::DotSegment),
            ("/../live", ProbePathError::DotSegment),
            ("/{probe}", ProbePathError::NonLiteralSegment),
            ("/{*path}", ProbePathError::NonLiteralSegment),
            ("/:probe", ProbePathError::NonLiteralSegment),
            ("/*path", ProbePathError::NonLiteralSegment),
            ("/live?full=1", ProbePathError::NonLiteralSegment),
            ("/live#details", ProbePathError::NonLiteralSegment),
            ("/health%2Flive", ProbePathError::NonLiteralSegment),
        ] {
            assert_eq!(ProbePath::new(invalid), Err(expected), "{invalid}");
        }
        let long = format!("/{}", "a".repeat(256));
        let leaked = Box::leak(long.into_boxed_str());
        assert_eq!(ProbePath::new(leaked), Err(ProbePathError::TooLong));
    }

    #[test]
    fn boundary_rejects_duplicate_probe_paths_before_axum_routing() {
        let second = Duration::from_secs(1);
        let request_policy = || {
            RequestPolicy::new(
                ShutdownHandle::new_unapproved().operation_admission(),
                ResponseConstructionBudget::new(second).unwrap(),
            )
        };
        let path = ProbePath::new("/health").unwrap();
        let duplicate_liveness = HttpBoundary::new(request_policy())
            .with_liveness(path)
            .unwrap()
            .with_liveness(path);
        assert!(matches!(
            duplicate_liveness,
            Err(ProbeRegistrationError::DuplicatePath)
        ));

        let health_policy = HealthPolicy::new(second, second, second * 3, second).unwrap();
        let monitor = HealthMonitor::new(health_policy, || async { Ok::<_, Infallible>(()) });
        let duplicate_cross_kind = HttpBoundary::new(request_policy())
            .with_liveness(path)
            .unwrap()
            .with_readiness(
                path,
                ReadinessPolicy::new(ShutdownHandle::new_unapproved().status(), monitor.reader()),
            );
        assert!(matches!(
            duplicate_cross_kind,
            Err(ProbeRegistrationError::DuplicatePath)
        ));
    }

    #[tokio::test]
    async fn boundary_rejects_guarded_probe_paths_without_polling_application_code() {
        use axum::routing::post;
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };

        let second = Duration::from_secs(1);
        let path = ProbePath::new("/health").unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let route_calls = calls.clone();
        let fallback_calls = calls.clone();
        let guarded = GuardedRouter::new()
            .route(
                "/health",
                post(move || {
                    route_calls.fetch_add(1, Ordering::SeqCst);
                    async { "guarded" }
                }),
            )
            .fallback(move || {
                fallback_calls.fetch_add(1, Ordering::SeqCst);
                async { "fallback" }
            });
        let result = HttpBoundary::new(RequestPolicy::new(
            ShutdownHandle::new_unapproved().operation_admission(),
            ResponseConstructionBudget::new(second).unwrap(),
        ))
        .with_liveness(path)
        .unwrap()
        .assemble(guarded)
        .await;

        assert!(matches!(
            result,
            Err(BoundaryAssemblyError::GuardedProbePath)
        ));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn boundary_rejects_nested_guarded_probe_paths_without_polling_application_code() {
        use axum::routing::post;
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };

        let second = Duration::from_secs(1);
        let path = ProbePath::new("/api/health").unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let route_calls = calls.clone();
        let nested = GuardedRouter::new().route(
            "/health",
            post(move || {
                route_calls.fetch_add(1, Ordering::SeqCst);
                async { "guarded" }
            }),
        );
        let guarded = GuardedRouter::new().nest("/api", nested);
        let result = HttpBoundary::new(RequestPolicy::new(
            ShutdownHandle::new_unapproved().operation_admission(),
            ResponseConstructionBudget::new(second).unwrap(),
        ))
        .with_liveness(path)
        .unwrap()
        .assemble(guarded)
        .await;

        assert!(matches!(
            result,
            Err(BoundaryAssemblyError::GuardedProbePath)
        ));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }
}
