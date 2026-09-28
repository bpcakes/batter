//! Library-owned HTTP composition with a fixed layer order.

use crate::{ReadinessPolicy, dependency_readiness, liveness, operational_http, serving};
use axum::{Router, middleware, routing::get};
use batter_core::{RegistrationError, registration::RegistrationTarget};
use group::DEFAULT_GROUP;
use std::{error::Error, fmt};
use tokio::net::TcpListener;

mod group;
mod guarded;
mod inventory;
mod pattern;
mod probe;
#[cfg(test)]
mod tests;

pub use group::{BrowserPolicy, GroupPolicy, RouteGroup, RouteGroupError};
pub use guarded::GuardedRouter;
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
    /// confirmed by routing a shared path through each pattern alone. If a
    /// request URI cannot preserve that path verbatim, assembly conservatively
    /// rejects the overlap before native merging.
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
