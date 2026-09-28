//! Library-owned HTTP composition with a fixed layer order.

use crate::{ReadinessPolicy, dependency_readiness, liveness, operational_http, serving};
use axum::{
    Router,
    extract::Request,
    handler::Handler,
    middleware,
    response::IntoResponse,
    routing::{MethodRouter, Route, get},
};
use batter_core::{RegistrationError, registration::RegistrationTarget};
use group::DEFAULT_GROUP;
use std::{convert::Infallible, error::Error, fmt};
use tokio::net::TcpListener;
use tower::{Layer, Service};

mod group;
mod inventory;
mod pattern;
mod probe;
#[cfg(test)]
mod tests;

pub use group::{BrowserPolicy, GroupPolicy, RouteGroup, RouteGroupError};
pub use probe::{ProbePath, ProbePathError, ProbeRegistrationError};

/// Sanitized failure to combine probes with guarded application routes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum BoundaryAssemblyError {
    /// A guarded route in any route group can match a path reserved for a
    /// public probe.
    GuardedProbePath,
    /// Routes of two route groups, named in declaration order, can match the
    /// same request path. The routes passed to `assemble` are named `default`.
    OverlappingGroupPaths {
        /// The group declared first.
        first: &'static str,
        /// The group declared later.
        second: &'static str,
    },
}

impl fmt::Display for BoundaryAssemblyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::GuardedProbePath => {
                formatter.write_str("guarded route conflicts with a probe path")
            }
            Self::OverlappingGroupPaths { first, second } => write!(
                formatter,
                "route groups {first} and {second} can match the same path"
            ),
        }
    }
}

impl Error for BoundaryAssemblyError {}

/// Guarded application routes whose identities remain visible to [`HttpBoundary`].
///
/// This is the canonical application-router builder. It mirrors the Axum
/// operations that preserve route identity while retaining every declared path
/// separately from the native router. That retained inventory lets boundary
/// assembly reject probe collisions and overlapping route groups without
/// executing application services. It also records whether a root or nested
/// fallback was declared, because only the default route group may own one.
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
    declares_fallback: bool,
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
            declares_fallback: false,
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
        self.declares_fallback |= nested.declares_fallback;
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
        self.declares_fallback |= other.declares_fallback;
        for path in other.route_patterns {
            if !self.route_patterns.contains(&path) {
                self.route_patterns.push(path);
            }
        }
        self
    }

    /// Set the guarded fallback without treating it as an explicit route identity.
    ///
    /// Only the default route group, passed to [`HttpBoundary::assemble`], can
    /// contain a root or nested fallback.
    pub fn fallback<H, T>(mut self, handler: H) -> Self
    where
        H: Handler<T, S>,
        T: 'static,
    {
        self.router = self.router.fallback(handler);
        self.declares_fallback = true;
        self
    }

    /// Set an infallible guarded fallback service for the default route group.
    pub fn fallback_service<T>(mut self, service: T) -> Self
    where
        T: Service<Request, Error = Infallible> + Clone + Send + Sync + 'static,
        T::Response: IntoResponse,
        T::Future: Send + 'static,
    {
        self.router = self.router.fallback_service(service);
        self.declares_fallback = true;
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
            declares_fallback: self.declares_fallback,
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
/// The assembled router has one fixed shape. Server correlation and the single
/// HTTP observer are outermost; probes sit outside every route group; and each
/// route group receives, from outside in, its private-response headers when it
/// has a [`BrowserPolicy`], its lifecycle admission and response-construction
/// deadline, its mutation checks when configured, and then the application's
/// own layers and routes. The routes passed to [`Self::assemble`] form the
/// `default` group, whose policy is given to [`Self::new`] and which owns every
/// root and nested fallback; [`Self::with_group`] adds named groups with their
/// own [`GroupPolicy`]. The caller supplies only policies, probe paths and
/// guarded routes; the order cannot be changed, no probe can end up inside an
/// admission gate, and no request path can reach routes of two groups. This is
/// the canonical path. [`crate::observe_http`], [`crate::request_admission`],
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
    policy: GroupPolicy,
    groups: Vec<RouteGroup>,
    probes: Router,
    probe_paths: Vec<ProbePath>,
}

impl HttpBoundary {
    /// Start a boundary whose default route group uses `policy`, with no named
    /// groups and no probes.
    ///
    /// A [`RequestPolicy`](crate::RequestPolicy) is a policy without browser
    /// policy; pass [`GroupPolicy::browser`] to give the default group one.
    pub fn new(policy: impl Into<GroupPolicy>) -> Self {
        Self {
            policy: policy.into(),
            groups: Vec::new(),
            probes: Router::new(),
            probe_paths: Vec::new(),
        }
    }

    /// Add a named route group with its own request and browser policy.
    ///
    /// The group is rejected before any routing when its name is invalid or
    /// already used (the routes passed to [`Self::assemble`] are `default`),
    /// when it declares no route, or when it declares a root or nested
    /// fallback. [`Self::assemble`] rejects groups that can match the same
    /// request path. See [`RouteGroup`] for a complete composition.
    pub fn with_group(mut self, group: RouteGroup) -> Result<Self, RouteGroupError> {
        group.validate()?;
        if group.name == DEFAULT_GROUP || self.groups.iter().any(|known| known.name == group.name) {
            return Err(RouteGroupError::DuplicateName(group.name));
        }
        self.groups.push(group);
        Ok(self)
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

    /// Validate probes and route groups, install each group's policy around its
    /// routes, merge the probes outside every group, and install correlation
    /// with the single HTTP observer outermost.
    ///
    /// `guarded` is the default route group. Unmatched paths, including those
    /// under a nested fallback, reach its fallback inside its admission; named
    /// groups keep unsupported methods on their routes inside their own
    /// policy. Routes added to the result afterward would sit outside the
    /// boundary, so the result is not a bare [`Router`].
    /// Route validation polls only library-owned inert inventories. It does not
    /// poll guarded handlers, fallbacks, or their middleware. A probe path
    /// reserves the complete route identity in every group, so even a different
    /// guarded method at that path is rejected before Axum merge can panic.
    /// Routes of different groups may not match one request path, even with
    /// different methods; overlap is decided from the retained patterns and
    /// confirmed by routing a shared path through each pattern alone.
    pub async fn assemble(
        self,
        guarded: GuardedRouter,
    ) -> Result<AssembledHttp, BoundaryAssemblyError> {
        let Self {
            policy,
            groups,
            probes,
            probe_paths,
        } = self;
        validate(&guarded, &groups, &probe_paths).await?;
        let mut router = probes;
        for group in groups {
            router = router.merge(group.policy.apply(group.routes.router));
        }
        // With two default fallbacks Axum retains the second router's; a second
        // custom fallback also supersedes a first default. Named groups declare
        // no fallback, so merging the default group last retains its layered one.
        let router = router
            .merge(policy.apply(guarded.router))
            .layer(middleware::from_fn(operational_http));
        Ok(AssembledHttp { router })
    }
}

/// Reject probe collisions in any group, then overlapping group paths.
async fn validate(
    default: &GuardedRouter,
    groups: &[RouteGroup],
    probes: &[ProbePath],
) -> Result<(), BoundaryAssemblyError> {
    let inventories: Vec<(&'static str, &[String])> =
        std::iter::once((DEFAULT_GROUP, default.route_patterns.as_slice()))
            .chain(
                groups
                    .iter()
                    .map(|group| (group.name, group.routes.route_patterns.as_slice())),
            )
            .collect();
    for &(_, patterns) in &inventories {
        if inventory::claims_probe(patterns, probes).await {
            return Err(BoundaryAssemblyError::GuardedProbePath);
        }
    }
    match inventory::overlapping_groups(&inventories).await {
        Some((first, second)) => {
            Err(BoundaryAssemblyError::OverlappingGroupPaths { first, second })
        }
        None => Ok(()),
    }
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
