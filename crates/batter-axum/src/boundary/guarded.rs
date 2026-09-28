//! Guarded application routes whose identities remain visible to assembly.

use super::{
    RouteInventory,
    declared::{DeclaredRouter, nested_pattern},
};
use axum::{
    Router,
    extract::Request,
    handler::Handler,
    response::IntoResponse,
    routing::{MethodRouter, Route},
};
use std::convert::Infallible;
use tower::{Layer, Service};

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
/// `GuardedRouter` instead. A router built by another router builder joins
/// through [`Self::from_router`] with a declared [`RouteInventory`].
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
///
/// [`HttpBoundary`]: super::HttpBoundary
#[must_use = "pass the guarded routes to HttpBoundary::assemble"]
pub struct GuardedRouter<S = ()> {
    pub(super) router: Router<S>,
    pub(super) route_patterns: Vec<String>,
    pub(super) declares_fallback: bool,
    pub(super) declared: Vec<DeclaredRouter<S>>,
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
            declared: Vec::new(),
        }
    }

    /// Admit a router built by another router builder, serving only its
    /// declared routes.
    ///
    /// A router converted from another builder, for example an OpenAPI router
    /// through `Router::from`, exposes no route inventory, so the application
    /// declares the patterns it serves. Awaited assembly routes a path of each
    /// declared pattern through a library-owned inspection copy of the router,
    /// whose every route and fallback only reports what matched, and returns
    /// [`BoundaryAssemblyError::RouteInventoryMismatch`] unless that path
    /// reaches the route registered with exactly that pattern. The same copy
    /// rejects any route of the router, declared or not, that matches a probe
    /// path. No application handler, fallback or middleware is called or
    /// polled.
    ///
    /// When serving, a request reaches the admitted router only if the router
    /// would route it to a declared pattern; every other request is routed as
    /// though the router were absent. An undeclared route or the router's own
    /// fallback therefore never serves a request, and group overlap is decided
    /// from the declared patterns: they must not share a request path with a
    /// probe, another group or another route set of their own group. The
    /// router then runs inside its group's policy like any guarded route, with
    /// native routing, path parameters and `MatchedPath`. Assembly prepares it
    /// once, as Axum prepares a router it serves, so its layers are built once
    /// and shared by all of its requests. [`Self::nest`], [`Self::merge`],
    /// [`Self::layer`], [`Self::route_layer`] and [`Self::with_state`] carry the
    /// admitted router and its inventory along.
    ///
    /// ```
    /// use axum::{Router, extract::Path, routing::get};
    /// use batter_axum::{
    ///     GuardedRouter, HttpBoundary, ProbePath, RequestPolicy, ResponseConstructionBudget,
    ///     RouteGroup, RouteInventory,
    /// };
    /// use batter_core::lifecycle::ShutdownHandle;
    /// use std::time::Duration;
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// // Stands in for a router converted from another router builder.
    /// let converted: Router = Router::new()
    ///     .route("/items", get(|| async { "items" }))
    ///     .route("/items/{id}", get(|Path(id): Path<u32>| async move { id.to_string() }));
    /// let items = GuardedRouter::from_router(
    ///     converted,
    ///     RouteInventory::new(["/items", "/items/{id}"])?,
    /// );
    /// let (control, approval) = ShutdownHandle::new_with_readiness_approval();
    /// approval.approve();
    /// let budget = ResponseConstructionBudget::new(Duration::from_secs(10))?;
    /// let policy = RequestPolicy::new(control.operation_admission(), budget);
    /// let assembled = HttpBoundary::new(policy.clone())
    ///     .with_liveness(ProbePath::new("/live")?)?
    ///     .with_group(RouteGroup::new("items", policy, items))?
    ///     .assemble(GuardedRouter::new().route("/work", get(|| async { "ok" })))
    ///     .await?;
    /// # let _ = assembled.into_router();
    /// # Ok(()) }
    /// ```
    ///
    /// [`BoundaryAssemblyError::RouteInventoryMismatch`]: super::BoundaryAssemblyError::RouteInventoryMismatch
    pub fn from_router(router: Router<S>, inventory: RouteInventory) -> Self {
        Self {
            declared: vec![DeclaredRouter::new(router, inventory)],
            ..Self::new()
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
            let full_path = nested_pattern(path, &nested_path);
            if !self.route_patterns.contains(&full_path) {
                self.route_patterns.push(full_path);
            }
        }
        self.declared.extend(
            nested
                .declared
                .into_iter()
                .map(|declared| declared.nest(path)),
        );
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
        self.declared.extend(other.declared);
        self
    }

    /// Set the guarded fallback without treating it as an explicit route identity.
    ///
    /// Only the default route group, passed to [`HttpBoundary::assemble`](super::HttpBoundary::assemble), can
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

    /// Apply a native Axum layer to every current guarded route and fallback,
    /// including every admitted router.
    pub fn layer<L>(mut self, layer: L) -> Self
    where
        L: Layer<Route> + Clone + Send + Sync + 'static,
        L::Service: Service<Request> + Clone + Send + Sync + 'static,
        <L::Service as Service<Request>>::Response: IntoResponse + 'static,
        <L::Service as Service<Request>>::Error: Into<Infallible> + 'static,
        <L::Service as Service<Request>>::Future: Send + 'static,
    {
        self.declared = self
            .declared
            .into_iter()
            .map(|declared| declared.map_router(|router| router.layer(layer.clone())))
            .collect();
        self.router = self.router.layer(layer);
        self
    }

    /// Apply a native Axum layer to every current explicit guarded route,
    /// including the routes of every admitted router.
    pub fn route_layer<L>(mut self, layer: L) -> Self
    where
        L: Layer<Route> + Clone + Send + Sync + 'static,
        L::Service: Service<Request> + Clone + Send + Sync + 'static,
        <L::Service as Service<Request>>::Response: IntoResponse + 'static,
        <L::Service as Service<Request>>::Error: Into<Infallible> + 'static,
        <L::Service as Service<Request>>::Future: Send + 'static,
    {
        // Axum rejects a route layer on a router without routes. An admitted
        // router without routes cannot pass assembly, so it is left unchanged.
        self.declared = self
            .declared
            .into_iter()
            .map(|declared| {
                declared.map_router(|router| {
                    if router.has_routes() {
                        router.route_layer(layer.clone())
                    } else {
                        router
                    }
                })
            })
            .collect();
        if self.declared.is_empty() || self.router.has_routes() {
            self.router = self.router.route_layer(layer);
        }
        self
    }

    /// Supply native Axum state while retaining the guarded route inventory.
    pub fn with_state<S2>(self, state: S) -> GuardedRouter<S2> {
        GuardedRouter {
            declared: self
                .declared
                .into_iter()
                .map(|declared| declared.map_router(|router| router.with_state(state.clone())))
                .collect(),
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
