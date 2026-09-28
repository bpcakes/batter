//! Guarded route groups with their own request and browser policy.

use super::GuardedRouter;
use crate::{
    RequestPolicy,
    browser::{MutationPolicy, MutationRejection, PrivateResponsePolicy},
    quota_observation::QuotaObservation,
    request_admission,
};
use axum::{
    Router,
    extract::{Request, State},
    http::{Method, request::Parts},
    middleware::{self, Next},
    response::Response,
};
use std::{error::Error, fmt, sync::Arc};

/// Name reserved for the routes passed to [`super::HttpBoundary::assemble`].
pub(super) const DEFAULT_GROUP: &str = "default";
const NAME_MAX_LEN: usize = 96;

type RejectionRenderer = dyn Fn(MutationRejection, &Parts) -> Response + Send + Sync;

/// Browser policy that [`HttpBoundary`](crate::HttpBoundary) installs for one route group.
///
/// Every browser policy applies one [`PrivateResponsePolicy`] to every response
/// the group returns, including admission, deadline and mutation rejections.
/// The constructor names the mutation choice: [`Self::with_mutation_checks`]
/// checks a [`MutationPolicy`] on every request whose method is not `GET`,
/// `HEAD`, `OPTIONS` or `TRACE`, and [`Self::without_mutation_checks`] states
/// that the group relies on application authentication instead. An empty
/// browser policy cannot be constructed.
///
/// The checks are browser signals, not authentication, authorization, CORS or
/// complete CSRF protection. Batter selects no rejection body: the renderer
/// maps the sanitized [`MutationRejection`] into the application's envelope and
/// receives the request metadata, including a
/// [`CorrelationId`](crate::CorrelationId) and the admitted
/// `OperationContext`, but not the body.
///
/// ```
/// use axum::response::IntoResponse;
/// use batter_axum::{
///     BrowserPolicy,
///     browser::{BrowserOrigin, MutationPolicy, PrivateResponsePolicy},
/// };
///
/// let origin = BrowserOrigin::https("https://app.example")?;
/// let account = BrowserPolicy::with_mutation_checks(
///     PrivateResponsePolicy::SameOriginReferrer,
///     MutationPolicy::exact_origin(origin),
///     |rejection, _parts| (rejection.status(), rejection.code()).into_response(),
/// );
/// let reports = BrowserPolicy::without_mutation_checks(PrivateResponsePolicy::NoReferrer);
/// # let _ = (account, reports);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
///
/// There is no empty or default browser policy:
///
/// ```compile_fail,E0599
/// let empty = batter_axum::BrowserPolicy::default();
/// ```
#[derive(Clone)]
#[must_use = "pass the browser policy to GroupPolicy::browser"]
pub struct BrowserPolicy {
    response: PrivateResponsePolicy,
    mutation: Option<Arc<MutationGuard>>,
}

struct MutationGuard {
    policy: MutationPolicy,
    render: Box<RejectionRenderer>,
}

impl BrowserPolicy {
    /// Apply private-response headers without checking browser mutation signals.
    ///
    /// Choose this only when the group's mutating routes do not accept ambient
    /// browser credentials, or the application authenticates them otherwise.
    pub fn without_mutation_checks(response: PrivateResponsePolicy) -> Self {
        Self {
            response,
            mutation: None,
        }
    }

    /// Apply private-response headers and check every unsafe-method request.
    ///
    /// `render` runs synchronously for a rejected request inside the group's
    /// admission and private-response headers, and must not block the runtime.
    pub fn with_mutation_checks<F>(
        response: PrivateResponsePolicy,
        mutation: MutationPolicy,
        render: F,
    ) -> Self
    where
        F: Fn(MutationRejection, &Parts) -> Response + Send + Sync + 'static,
    {
        Self {
            response,
            mutation: Some(Arc::new(MutationGuard {
                policy: mutation,
                render: Box::new(render),
            })),
        }
    }
}

/// Request policy and optional browser policy for one guarded route group.
///
/// A [`RequestPolicy`] converts into a group policy without browser policy, so
/// `HttpBoundary::new(request_policy)` keeps its meaning.
#[derive(Clone)]
#[must_use = "pass the group policy to HttpBoundary::new or RouteGroup::new"]
pub struct GroupPolicy {
    request: RequestPolicy,
    browser: Option<BrowserPolicy>,
}

impl GroupPolicy {
    /// Admit the group's requests with `request` and add no browser policy.
    pub fn new(request: RequestPolicy) -> Self {
        Self {
            request,
            browser: None,
        }
    }

    /// Admit the group's requests with `request` and apply `browser` around them.
    pub fn browser(request: RequestPolicy, browser: BrowserPolicy) -> Self {
        Self {
            request,
            browser: Some(browser),
        }
    }

    /// Install this policy around every route and fallback in `router`.
    ///
    /// Layers added later run outside earlier ones, so the order is: private
    /// response headers, then admission, then mutation checks, then the
    /// application's own layers and handler.
    pub(super) fn apply(self, mut router: Router) -> Router {
        let Self { request, browser } = self;
        if let Some(guard) = browser
            .as_ref()
            .and_then(|browser| browser.mutation.clone())
        {
            router = router.layer(middleware::from_fn_with_state(guard, check_mutation));
        }
        router = router.layer(middleware::from_fn_with_state(request, request_admission));
        match browser {
            Some(browser) => router.layer(browser.response),
            None => router,
        }
    }
}

impl From<RequestPolicy> for GroupPolicy {
    fn from(request: RequestPolicy) -> Self {
        Self::new(request)
    }
}

// The boundary's outermost `operational_http` already scopes the dispatch for
// polling and destruction of this nested future.
async fn check_mutation(
    State(guard): State<Arc<MutationGuard>>,
    request: Request,
    next: Next,
) -> Response {
    if is_safe(request.method()) {
        return next.run(request).await;
    }
    match guard.policy.check(request.headers()) {
        Ok(()) => next.run(request).await,
        Err(rejection) => {
            let (mut parts, body) = request.into_parts();
            drop(body);
            // The renderer needs request metadata, never the observer's private writer.
            parts.extensions.remove::<QuotaObservation>();
            (guard.render)(rejection, &parts)
        }
    }
}

/// RFC 9110 safe methods; every other method, including extensions, is checked.
fn is_safe(method: &Method) -> bool {
    matches!(
        *method,
        Method::GET | Method::HEAD | Method::OPTIONS | Method::TRACE
    )
}

/// Named guarded routes with their own [`GroupPolicy`].
///
/// Add a group with [`HttpBoundary::with_group`](crate::HttpBoundary::with_group).
/// The routes passed to `assemble` form the group named `default`, which alone
/// owns fallbacks. A named group must declare at least one route and no root or
/// nested fallback; its unmatched paths reach the default group's fallback,
/// and unsupported methods on its routes stay inside its own layers. Assembly
/// rejects route groups whose routes can match the same request path, even
/// with different methods.
///
/// ```
/// use axum::{
///     Extension,
///     response::IntoResponse,
///     routing::{get, post},
/// };
/// use batter_core::{lifecycle::ShutdownHandle, operation::OperationContext};
/// use batter_axum::{
///     BrowserPolicy, GroupPolicy, GuardedRouter, HttpBoundary, ProbePath, RequestPolicy,
///     ResponseConstructionBudget, RouteGroup,
///     browser::{BrowserOrigin, MutationPolicy, PrivateResponsePolicy},
/// };
/// use std::time::Duration;
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let (control, approval) = ShutdownHandle::new_with_readiness_approval();
/// approval.approve();
/// let policy = |seconds| -> Result<RequestPolicy, batter_core::ConfigurationError> {
///     let budget = ResponseConstructionBudget::new(Duration::from_secs(seconds))?;
///     Ok(RequestPolicy::new(control.operation_admission(), budget))
/// };
/// let uploads = GuardedRouter::new().route("/uploads", post(|| async { "stored" }));
/// let account = GuardedRouter::new().route(
///     "/account",
///     get(|| async { "profile" }).post(
///         |Extension(context): Extension<OperationContext>| async move {
///             context.check().expect("admitted context");
///             "saved"
///         },
///     ),
/// );
/// let browser = BrowserPolicy::with_mutation_checks(
///     PrivateResponsePolicy::SameOriginReferrer,
///     MutationPolicy::exact_origin(BrowserOrigin::https("https://app.example")?),
///     |rejection, _parts| (rejection.status(), rejection.code()).into_response(),
/// );
/// let assembled = HttpBoundary::new(policy(10)?)
///     .with_liveness(ProbePath::new("/live")?)?
///     .with_group(RouteGroup::new("uploads", policy(90)?, uploads))?
///     .with_group(RouteGroup::new(
///         "account",
///         GroupPolicy::browser(policy(10)?, browser),
///         account,
///     ))?
///     .assemble(GuardedRouter::new().route("/work", get(|| async { "ok" })))
///     .await?;
/// # let _ = assembled.into_router();
/// # Ok(()) }
/// ```
///
/// A bare Axum router exposes no route inventory, so it cannot join a group:
///
/// ```compile_fail,E0308
/// # use batter_axum::{RequestPolicy, RouteGroup};
/// # fn invalid(policy: RequestPolicy) {
/// let group = RouteGroup::new("uploads", policy, axum::Router::new());
/// # }
/// ```
#[must_use = "add the route group with HttpBoundary::with_group"]
pub struct RouteGroup {
    pub(super) name: &'static str,
    pub(super) policy: GroupPolicy,
    pub(super) routes: GuardedRouter,
}

impl RouteGroup {
    /// Name guarded routes and select the policy installed around them.
    ///
    /// Names are 1–96 ASCII alphanumeric, `.`, `_` or `-` bytes and identify
    /// the group in sanitized assembly errors.
    pub fn new(name: &'static str, policy: impl Into<GroupPolicy>, routes: GuardedRouter) -> Self {
        Self {
            name,
            policy: policy.into(),
            routes,
        }
    }

    pub(super) fn validate(&self) -> Result<(), RouteGroupError> {
        if self.name.is_empty()
            || self.name.len() > NAME_MAX_LEN
            || !self
                .name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
        {
            return Err(RouteGroupError::InvalidName);
        }
        if self.routes.route_patterns.is_empty() {
            return Err(RouteGroupError::NoRoutes);
        }
        if self.routes.declares_fallback {
            return Err(RouteGroupError::DeclaresFallback);
        }
        Ok(())
    }
}

/// Sanitized failure to add a [`RouteGroup`] to an [`HttpBoundary`](crate::HttpBoundary).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum RouteGroupError {
    /// Names must be 1–96 ASCII alphanumeric, `.`, `_` or `-` bytes.
    InvalidName,
    /// Another group in this boundary already uses the name; the routes passed
    /// to `assemble` are the group named `default`.
    DuplicateName(&'static str),
    /// The group declares no guarded route.
    NoRoutes,
    /// The group declares a root or nested fallback; only the default group
    /// owns fallbacks.
    DeclaresFallback,
}

impl fmt::Display for RouteGroupError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidName => formatter.write_str("route group name is invalid"),
            Self::DuplicateName(name) => {
                write!(formatter, "route group name is already used: {name}")
            }
            Self::NoRoutes => formatter.write_str("route group declares no route"),
            Self::DeclaresFallback => {
                formatter.write_str("only the default route group can declare a fallback")
            }
        }
    }
}

impl Error for RouteGroupError {}
