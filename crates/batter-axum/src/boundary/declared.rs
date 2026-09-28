//! Routers built outside `GuardedRouter`, admitted through a declared route inventory.

use super::{
    ProbePath,
    pattern::{self, Segment},
};
use axum::{
    Router,
    body::Body,
    extract::{MatchedPath, Request},
    http::{StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::Route,
};
use std::{
    collections::HashSet,
    convert::Infallible,
    error::Error,
    fmt,
    future::{Future, Ready, ready},
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};
use tower::{Service, ServiceExt, layer::layer_fn};

/// Capture and wildcard filler for inventory witnesses. Route literals contain
/// braces only when escaped, so an ordinary literal route cannot take priority
/// over the declared pattern for such a path.
const FILLER: &str = "{}";

/// The route patterns a router built outside [`GuardedRouter`] may serve.
///
/// Pass the inventory with the router to [`GuardedRouter::from_router`]. Each
/// pattern uses Axum 0.8 route syntax and is spelled exactly as the router
/// registered it, including any nesting prefix applied inside that router.
/// Repeated patterns are kept once. Construction validates only the syntax,
/// including the parameter names Axum's router accepts;
/// [`HttpBoundary::assemble`] checks every pattern against the router.
///
/// ```
/// use batter_axum::{RouteInventory, RouteInventoryError};
///
/// let inventory = RouteInventory::new(["/items", "/items/{id}"])?;
/// # let _ = inventory;
/// assert_eq!(
///     RouteInventory::new(Vec::<String>::new()),
///     Err(RouteInventoryError::Empty)
/// );
/// assert_eq!(
///     RouteInventory::new(["/items", "items/{id}"]),
///     Err(RouteInventoryError::InvalidPattern { index: 1 })
/// );
/// # Ok::<(), RouteInventoryError>(())
/// ```
///
/// [`GuardedRouter`]: super::GuardedRouter
/// [`GuardedRouter::from_router`]: super::GuardedRouter::from_router
/// [`HttpBoundary::assemble`]: super::HttpBoundary::assemble
#[derive(Clone, Debug, Eq, PartialEq)]
#[must_use = "pass the route inventory to GuardedRouter::from_router"]
pub struct RouteInventory {
    patterns: Vec<String>,
}

impl RouteInventory {
    /// Validate a non-empty sequence of absolute Axum route patterns.
    pub fn new<I>(patterns: I) -> Result<Self, RouteInventoryError>
    where
        I: IntoIterator,
        I::Item: Into<String>,
    {
        let mut declared: Vec<String> = Vec::new();
        for (index, pattern) in patterns.into_iter().enumerate() {
            let pattern = pattern.into();
            if pattern::parse(&pattern).is_none() {
                return Err(RouteInventoryError::InvalidPattern { index });
            }
            if !declared.contains(&pattern) {
                declared.push(pattern);
            }
        }
        if declared.is_empty() {
            return Err(RouteInventoryError::Empty);
        }
        Ok(Self { patterns: declared })
    }
}

/// Sanitized failure to construct a [`RouteInventory`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum RouteInventoryError {
    /// The inventory declares no route pattern.
    Empty,
    /// A pattern does not start with `/` or is not valid Axum 0.8 route syntax.
    InvalidPattern {
        /// Zero-based position of the rejected pattern in the supplied sequence.
        index: usize,
    },
}

impl fmt::Display for RouteInventoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("route inventory declares no route"),
            Self::InvalidPattern { index } => {
                write!(formatter, "route inventory pattern {index} is invalid")
            }
        }
    }
}

impl Error for RouteInventoryError {}

/// A router built outside `GuardedRouter` with the patterns it may serve.
pub(super) struct DeclaredRouter<S> {
    pub(super) router: Router<S>,
    pub(super) patterns: Vec<String>,
}

impl<S> DeclaredRouter<S>
where
    S: Clone + Send + Sync + 'static,
{
    pub(super) fn new(router: Router<S>, inventory: RouteInventory) -> Self {
        Self {
            router,
            patterns: inventory.patterns,
        }
    }

    /// Nest under `path`, prefixing every declared pattern as Axum prefixes its route.
    pub(super) fn nest(self, path: &str) -> Self {
        Self {
            // The guarded router's own nest has already validated `path`; keep
            // the route-syntax configuration the admitted router was built with.
            router: Router::new().without_v07_checks().nest(path, self.router),
            patterns: self
                .patterns
                .iter()
                .map(|pattern| nested_pattern(path, pattern))
                .collect(),
        }
    }

    /// Transform the router while keeping its declared patterns.
    pub(super) fn map_router<S2>(
        self,
        map: impl FnOnce(Router<S>) -> Router<S2>,
    ) -> DeclaredRouter<S2> {
        DeclaredRouter {
            router: map(self.router),
            patterns: self.patterns,
        }
    }
}

/// Join a nesting prefix and a nested route pattern exactly as Axum's `nest` does.
pub(super) fn nested_pattern(prefix: &str, nested: &str) -> String {
    if prefix.ends_with('/') {
        format!("{prefix}{}", nested.trim_start_matches('/'))
    } else if nested == "/" {
        prefix.to_owned()
    } else {
        format!("{prefix}{nested}")
    }
}

/// Response extension naming the explicit route an inspected request reached.
#[derive(Clone)]
struct ExplicitRoute(Option<MatchedPath>);

/// Library-owned endpoint that reports the match and drops the service it replaced.
#[derive(Clone, Copy)]
struct Report {
    explicit: bool,
}

impl Service<Request> for Report {
    type Response = Response;
    type Error = Infallible;
    type Future = Ready<Result<Response, Infallible>>;

    fn poll_ready(&mut self, _context: &mut Context<'_>) -> Poll<Result<(), Infallible>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, request: Request) -> Self::Future {
        let mut response = StatusCode::NO_CONTENT.into_response();
        if self.explicit {
            let matched = request.extensions().get::<MatchedPath>().cloned();
            response.extensions_mut().insert(ExplicitRoute(matched));
        }
        ready(Ok(response))
    }
}

/// A copy of an admitted router whose every endpoint only reports what matched.
///
/// Axum's `Router::layer` replaces every path route including its method
/// fallback, every fallback route and the catch-all fallback; `route_layer`
/// then replaces the path routes again, outermost. Axum still matches the path
/// and records `MatchedPath`, but no application handler, fallback or
/// middleware service is called or polled. The copy is prepared once, so
/// routing through it never rebuilds the layers it replaced.
#[derive(Clone)]
pub(super) struct Inspection(Router);

impl Inspection {
    pub(super) fn new(router: &Router) -> Self {
        let reporting = router
            .clone()
            .layer(layer_fn(|_replaced: Route| Report { explicit: false }));
        let reporting = if reporting.has_routes() {
            reporting.route_layer(layer_fn(|_replaced: Route| Report { explicit: true }))
        } else {
            reporting
        };
        Self(reporting.with_state(()))
    }

    /// The explicit route reached by `uri`, or `None` when only a fallback matches.
    async fn reached(&self, uri: Uri) -> Option<Option<MatchedPath>> {
        let mut request = Request::new(Body::empty());
        *request.uri_mut() = uri;
        let response = self
            .0
            .clone()
            .oneshot(request)
            .await
            .expect("Axum routers are infallible services");
        response
            .extensions()
            .get::<ExplicitRoute>()
            .map(|route| route.0.clone())
    }

    /// Whether any route of the router, declared or not, matches a probe path.
    pub(super) async fn claims_probe(&self, probes: &[ProbePath]) -> bool {
        for probe in probes {
            if self
                .reached(Uri::from_static(probe.as_str()))
                .await
                .is_some()
            {
                return true;
            }
        }
        false
    }

    /// Whether a path of every declared pattern reaches the route registered
    /// with exactly that pattern.
    pub(super) async fn serves_inventory(&self, patterns: &[String]) -> bool {
        for pattern in patterns {
            if !self.serves_pattern(pattern).await {
                return false;
            }
        }
        true
    }

    /// Fail closed when the witness cannot travel verbatim in a request URI:
    /// no request can then reach the pattern's own route.
    async fn serves_pattern(&self, pattern: &str) -> bool {
        let Some(witness) = witness(pattern) else {
            return false;
        };
        let Ok(uri) = Uri::try_from(witness.as_str()) else {
            return false;
        };
        uri.path() == witness
            && self
                .reached(uri)
                .await
                .flatten()
                .is_some_and(|matched| matched.as_str() == pattern)
    }
}

/// A path that `pattern` itself matches, with every capture and wildcard
/// remainder filled by [`FILLER`].
///
/// matchit 0.8.4 refuses a capture beside a wildcard in the same position, so
/// only a literal or a capture prefix containing braces, which a route can
/// spell only with `{{` or `}}`, could take such a path from the pattern.
fn witness(pattern: &str) -> Option<String> {
    let mut path = Vec::new();
    for segment in pattern::parse(pattern)? {
        path.push(b'/');
        match segment {
            Segment::Literal(literal) => path.extend_from_slice(&literal),
            Segment::Capture(prefix) | Segment::Wildcard(prefix) => {
                path.extend_from_slice(&prefix);
                path.extend_from_slice(FILLER.as_bytes());
            }
        }
    }
    String::from_utf8(path).ok()
}

/// An admitted router served inside its group's policy and the observer.
pub(super) struct Admitted {
    inspection: Inspection,
    patterns: HashSet<String>,
    served: Router,
}

impl Admitted {
    /// Prepare `served` once, as Axum prepares a router it serves, so each
    /// request reuses the layers built here instead of rebuilding them.
    pub(super) fn new(inspection: Inspection, patterns: Vec<String>, served: Router) -> Self {
        Self {
            inspection,
            patterns: patterns.into_iter().collect(),
            served: served.with_state(()),
        }
    }

    /// Whether the admitted router would route `uri` to a declared pattern.
    async fn serves(&self, uri: &Uri) -> bool {
        match self.inspection.reached(uri.clone()).await {
            Some(Some(matched)) => self.patterns.contains(matched.as_str()),
            Some(None) | None => false,
        }
    }
}

/// The root fallback of a boundary with admitted routers.
///
/// A request arrives here only when no native route matched it, through the
/// capture-free root fallback routes, so it carries no path parameters or
/// matched path. It goes unchanged to the admitted router that routes it to a
/// declared pattern, if one does, and otherwise to the default group's
/// fallbacks. Those sit behind this service because a nested fallback matched
/// first would add its path captures to the request.
#[derive(Clone)]
pub(super) struct Dispatch {
    admitted: Arc<[Admitted]>,
    fallbacks: Router,
}

impl Dispatch {
    /// Prepare `fallbacks` once, like the admitted routers.
    pub(super) fn new(admitted: Vec<Admitted>, fallbacks: Router) -> Self {
        Self {
            admitted: admitted.into(),
            fallbacks: fallbacks.with_state(()),
        }
    }
}

impl Service<Request> for Dispatch {
    type Response = Response;
    type Error = Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Response, Infallible>> + Send>>;

    fn poll_ready(&mut self, _context: &mut Context<'_>) -> Poll<Result<(), Infallible>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, request: Request) -> Self::Future {
        let Self {
            admitted,
            fallbacks,
        } = self.clone();
        Box::pin(async move {
            for router in admitted.iter() {
                if router.serves(request.uri()).await {
                    return router.served.clone().oneshot(request).await;
                }
            }
            fallbacks.oneshot(request).await
        })
    }
}
