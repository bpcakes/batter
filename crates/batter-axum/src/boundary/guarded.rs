//! Guarded application routes whose identities remain visible to assembly.

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
///
/// [`HttpBoundary`]: super::HttpBoundary
#[must_use = "pass the guarded routes to HttpBoundary::assemble"]
pub struct GuardedRouter<S = ()> {
    pub(super) router: Router<S>,
    pub(super) route_patterns: Vec<String>,
    pub(super) declares_fallback: bool,
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
