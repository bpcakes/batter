//! Library-owned HTTP composition with a fixed layer order.

use crate::{ReadinessDecision, ReadinessPolicy, dependency_readiness, liveness, serving};
use assembly::Assembling;
use axum::{Router, http::request::Parts, response::Response, routing::get};
use batter_core::{RegistrationError, registration::RegistrationTarget};
use group::DEFAULT_GROUP;
use std::{error::Error, fmt, sync::Arc};
use tokio::net::TcpListener;

mod assembly;
mod declared;
mod group;
mod guarded;
mod inventory;
mod pattern;
mod probe;
#[cfg(test)]
mod tests;

pub use declared::{RouteInventory, RouteInventoryError};
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
    /// A router admitted with [`GuardedRouter::from_router`] does not serve a
    /// pattern of its [`RouteInventory`]: no path of the pattern reaches the
    /// route registered with exactly that pattern.
    RouteInventoryMismatch {
        /// The group containing the admitted router.
        group: &'static str,
    },
    /// Declared routes of a router admitted with [`GuardedRouter::from_router`]
    /// can match the same request path as other routes of the same group.
    OverlappingRouteInventory {
        /// The group containing the overlapping routes.
        group: &'static str,
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
            Self::RouteInventoryMismatch { group } => write!(
                formatter,
                "route inventory in group {group} does not match its router"
            ),
            Self::OverlappingRouteInventory { group } => write!(
                formatter,
                "route inventory in group {group} can match the same path as other routes of that group"
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
/// own [`GroupPolicy`]. The caller supplies only policies, probe paths, optional
/// probe renderers and guarded routes; the order cannot be changed, no probe
/// can end up inside an admission gate, and no request path can reach routes
/// of two groups. A probe renderer chooses the probe's body and headers, never
/// its status or readiness decision. This is the canonical path.
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

    /// Mount the process liveness probe at a validated literal path outside
    /// admission, answering an empty 200. [`Self::with_rendered_liveness`]
    /// answers with the application's body instead.
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

    /// Mount the process liveness probe at a validated literal path outside
    /// admission, with the application's response.
    ///
    /// `render` receives the request metadata, including the generated
    /// [`CorrelationId`](crate::CorrelationId) but not the body or private
    /// quota/observation/operational ownership state, and returns
    /// the application's response, for example a documented JSON body and its
    /// headers. Answering at all is the liveness signal, so the boundary then
    /// sets status 200, replacing any status the renderer chose, and removes
    /// any [`HttpObservationLevel`](crate::HttpObservationLevel) override, so
    /// the completion event keeps the empty-body probe's default INFO. The
    /// path is reserved and checked exactly as for [`Self::with_liveness`].
    /// `render` runs synchronously for every probe request and must not block
    /// the runtime. See [`Self::with_rendered_readiness`] for an example.
    pub fn with_rendered_liveness<F>(
        mut self,
        path: ProbePath,
        render: F,
    ) -> Result<Self, ProbeRegistrationError>
    where
        F: Fn(&Parts) -> Response + Send + Sync + 'static,
    {
        self.reserve_probe(path)?;
        self.probes = self
            .probes
            .route(path.as_str(), probe::rendered_liveness(Arc::new(render)));
        Ok(self)
    }

    /// Mount lifecycle-plus-dependency readiness at a validated literal path
    /// outside admission, answering an empty 200 or 503.
    /// [`Self::with_rendered_readiness`] answers with the application's body
    /// instead.
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

    /// Mount lifecycle-plus-dependency readiness at a validated literal path
    /// outside admission, with the application's response.
    ///
    /// Each request takes one fresh decision from `policy`, including any
    /// application conditions added with [`ReadinessPolicy::with_condition`],
    /// and passes it to `render` with the request metadata, including the
    /// generated [`CorrelationId`](crate::CorrelationId) but not the body or
    /// private quota/observation/operational ownership state. Cloned renderer
    /// metadata cannot rewrite the original request's observation on redispatch.
    /// `render` chooses the body and headers, for example an
    /// OpenAPI-documented JSON document, and cannot alter the decision: the
    /// boundary then sets the status from [`crate::readiness_status`], 200 only
    /// for [`ReadinessDecision::Ready`] and 503 otherwise, and replaces the
    /// [`ReadinessDecision`] and [`HttpObservationLevel`](crate::HttpObservationLevel)
    /// response extensions with the decision and the policy's severity,
    /// whatever the renderer set. The path is reserved and checked exactly as
    /// for [`Self::with_readiness`]. `render` runs synchronously for every probe
    /// request and must not block the runtime.
    ///
    /// ```
    /// use axum::{
    ///     Json,
    ///     http::request::Parts,
    ///     response::{IntoResponse, Response},
    ///     routing::get,
    /// };
    /// use batter_axum::{
    ///     CorrelationId, GuardedRouter, HttpBoundary, ProbePath, ReadinessDecision,
    ///     ReadinessPolicy, RequestPolicy, ResponseConstructionBudget,
    /// };
    /// use batter_core::{
    ///     health::{HealthMonitor, HealthPolicy},
    ///     lifecycle::ShutdownHandle,
    ///     readiness::{ReadinessCondition, ReadinessUnreadyReason},
    /// };
    /// use serde::Serialize;
    /// use std::{
    ///     convert::Infallible,
    ///     sync::{Arc, atomic::{AtomicBool, Ordering}},
    ///     time::Duration,
    /// };
    ///
    /// /// The application's documented probe body.
    /// #[derive(Serialize)]
    /// struct ProbeBody {
    ///     status: &'static str,
    ///     request_id: Option<String>,
    /// }
    ///
    /// fn probe_body(status: &'static str, parts: &Parts) -> Response {
    ///     let request_id = parts.extensions.get::<CorrelationId>().map(|id| id.to_string());
    ///     Json(ProbeBody { status, request_id }).into_response()
    /// }
    ///
    /// fn readiness_body(decision: ReadinessDecision, parts: &Parts) -> Response {
    ///     let status = match decision {
    ///         ReadinessDecision::Ready => "ready",
    ///         ReadinessDecision::Unready(ReadinessUnreadyReason::Condition(condition)) => {
    ///             condition.as_str()
    ///         }
    ///         ReadinessDecision::Unready(_) => "unavailable",
    ///     };
    ///     probe_body(status, parts)
    /// }
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let (control, approval) = ShutdownHandle::new_with_readiness_approval();
    /// approval.approve();
    /// let second = Duration::from_secs(1);
    /// let health = HealthPolicy::new(second, second, second * 3, second)?;
    /// let monitor = HealthMonitor::new(health, || async { Ok::<_, Infallible>(()) });
    /// // Application state that the application's own tasks keep current.
    /// let leases_valid = Arc::new(AtomicBool::new(false));
    /// let leases = leases_valid.clone();
    /// let readiness = ReadinessPolicy::new(control.status(), monitor.reader())
    ///     .with_condition(ReadinessCondition::new("key-leases")?, move || {
    ///         leases.load(Ordering::Acquire)
    ///     });
    /// let budget = ResponseConstructionBudget::new(second)?;
    /// let assembled = HttpBoundary::new(RequestPolicy::new(control.operation_admission(), budget))
    ///     .with_rendered_liveness(ProbePath::new("/live")?, |parts| probe_body("live", parts))?
    ///     .with_rendered_readiness(ProbePath::new("/ready")?, readiness, readiness_body)?
    ///     .assemble(GuardedRouter::new().route("/work", get(|| async { "ok" })))
    ///     .await?;
    /// // Inside protected startup: assembled.register_in(scope, "http", listener)?;
    /// # let _ = (assembled.into_router(), leases_valid);
    /// # drop(monitor);
    /// # Ok(()) }
    /// ```
    pub fn with_rendered_readiness<E, F>(
        mut self,
        path: ProbePath,
        policy: ReadinessPolicy<E>,
        render: F,
    ) -> Result<Self, ProbeRegistrationError>
    where
        E: Send + Sync + 'static,
        F: Fn(ReadinessDecision, &Parts) -> Response + Send + Sync + 'static,
    {
        self.reserve_probe(path)?;
        self.probes = self.probes.route(
            path.as_str(),
            probe::rendered_readiness(policy, Arc::new(render)),
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
    /// Route validation polls only library-owned inert inventories and
    /// inspection copies. It does not poll guarded handlers, fallbacks, or
    /// their middleware. A probe path reserves the complete route identity in
    /// every group, so even a different guarded method at that path is
    /// rejected before Axum merge can panic.
    /// Routes of different groups may not match one request path, even with
    /// different methods; overlap is decided from the retained patterns and
    /// confirmed by routing a shared path through each pattern alone. If a
    /// request URI cannot preserve that path verbatim, assembly conservatively
    /// rejects the overlap before native merging. Routers admitted with
    /// [`GuardedRouter::from_router`] are checked against their
    /// [`RouteInventory`] first, their declared patterns take part in the
    /// overlap decision, and they are never merged into the native router.
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
        let groups = std::iter::once(Assembling::new(DEFAULT_GROUP, policy, guarded))
            .chain(
                groups
                    .into_iter()
                    .map(|group| Assembling::new(group.name, group.policy, group.routes)),
            )
            .collect();
        let router = assembly::assemble(probes, &probe_paths, groups).await?;
        Ok(AssembledHttp { router })
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
