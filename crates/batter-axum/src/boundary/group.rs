//! Guarded route groups with their own request and browser policy.

use super::GuardedRouter;
use crate::{
    RequestPolicy,
    browser::{
        CredentialAccess, MutationPolicy, MutationRejection, PrivateResponsePolicy,
        record_credential_access,
    },
    correlation,
    low_level::request_admission,
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
/// in a [`MutationCheckWaiver`] why the group needs no check. An empty browser
/// policy cannot be constructed.
///
/// The choice also decides which requests may present ambient browser
/// credentials. Handlers read cookies only through
/// [`BrowserCredentials`](crate::browser::BrowserCredentials), which the
/// group's policy grants on every safe-method request, on a mutation that
/// passed its checks, and on a mutation the application declared it verifies
/// itself, but refuses on a mutation in a group whose mutating routes accept
/// no ambient credentials. Cookie sessions on an unchecked group are therefore
/// a visible waiver, not a silent default.
///
/// The checks are browser signals, not authentication, authorization, CORS or
/// complete CSRF protection. Batter selects no rejection body: the renderer
/// maps the sanitized [`MutationRejection`] into the application's envelope and
/// receives the request metadata, including a
/// [`CorrelationId`](crate::CorrelationId) and the admitted
/// `OperationContext` extension, but not the body, the private
/// quota/observation/operational ownership state or the admission record.
/// Redispatching cloned metadata cannot mutate the original request's
/// observation, and the new request is not admitted without admission of its own.
///
/// ```
/// use axum::response::IntoResponse;
/// use batter_axum::{
///     BrowserPolicy, MutationCheckWaiver,
///     browser::{BrowserOrigin, MutationPolicy, PrivateResponsePolicy},
/// };
///
/// let origin = BrowserOrigin::https("https://app.example")?;
/// let account = BrowserPolicy::with_mutation_checks(
///     PrivateResponsePolicy::SameOriginReferrer,
///     MutationPolicy::exact_origin(origin),
///     |rejection, _parts| (rejection.status(), rejection.code()).into_response(),
/// );
/// let reports = BrowserPolicy::without_mutation_checks(
///     PrivateResponsePolicy::NoReferrer,
///     MutationCheckWaiver::NoAmbientCredentials,
/// );
/// # let _ = (account, reports);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
///
/// There is no empty or default browser policy:
///
/// ```compile_fail,E0599
/// let empty = batter_axum::BrowserPolicy::default();
/// ```
///
/// and no unchecked policy that does not state its waiver:
///
/// ```compile_fail,E0061
/// use batter_axum::{BrowserPolicy, browser::PrivateResponsePolicy};
///
/// let unstated = BrowserPolicy::without_mutation_checks(PrivateResponsePolicy::NoReferrer);
/// ```
#[derive(Clone)]
#[must_use = "pass the browser policy to GroupPolicy::browser"]
pub struct BrowserPolicy {
    response: PrivateResponsePolicy,
    mutation: MutationGate,
}

/// What a browser group does with a request whose method is not safe.
#[derive(Clone)]
enum MutationGate {
    Checked(Arc<MutationGuard>),
    Waived(MutationCheckWaiver),
}

struct MutationGuard {
    policy: MutationPolicy,
    render: Box<RejectionRenderer>,
}

/// Why a route group's browser policy checks no browser mutation signals.
///
/// [`BrowserPolicy::without_mutation_checks`] requires one, so every unchecked
/// browser group records its reason at the call site, and the reason decides
/// whether the group's mutations may present ambient browser credentials
/// through [`BrowserCredentials`](crate::browser::BrowserCredentials).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MutationCheckWaiver {
    /// The group's mutating routes accept no ambient browser credentials: they
    /// authenticate with credentials a cross-site page cannot make the browser
    /// attach, such as a bearer token.
    ///
    /// Batter enforces the claim for cookies it presents: extracting
    /// [`BrowserCredentials`](crate::browser::BrowserCredentials) on any method
    /// other than `GET`, `HEAD`, `OPTIONS` or `TRACE` in this group answers
    /// [`BrowserCredentialsRejection::UncheckedMutation`](crate::browser::BrowserCredentialsRejection::UncheckedMutation),
    /// a sanitized 500. Safe-method requests still present cookies.
    NoAmbientCredentials,
    /// The application verifies every mutating request in the group that
    /// presents ambient credentials through its own protocol, for example a
    /// synchronizer token it issues and compares, or a cross-site form post
    /// that it verifies against its own state value.
    ///
    /// Mutations in this group present cookies through
    /// [`BrowserCredentials`](crate::browser::BrowserCredentials) without any
    /// Batter check. The verification is application policy that Batter
    /// neither performs nor observes; prefer
    /// [`BrowserPolicy::with_mutation_checks`] whenever a [`MutationPolicy`]
    /// can express the request's legitimate origin.
    ApplicationChecksMutations,
}

impl BrowserPolicy {
    /// Apply private-response headers without checking browser mutation
    /// signals, for the reason `waiver` states.
    ///
    /// With [`MutationCheckWaiver::NoAmbientCredentials`] the group's mutations
    /// cannot read cookies through
    /// [`BrowserCredentials`](crate::browser::BrowserCredentials); with
    /// [`MutationCheckWaiver::ApplicationChecksMutations`] they can, and the
    /// application owns their verification.
    ///
    /// Migration: the one-argument `without_mutation_checks(response)` now
    /// takes the waiver as its second argument. Pass
    /// `MutationCheckWaiver::NoAmbientCredentials` when the group's mutating
    /// routes read no cookie; when they authenticate with a cookie, prefer
    /// [`Self::with_mutation_checks`], and pass
    /// `MutationCheckWaiver::ApplicationChecksMutations` only when the
    /// application verifies those requests itself.
    pub fn without_mutation_checks(
        response: PrivateResponsePolicy,
        waiver: MutationCheckWaiver,
    ) -> Self {
        Self {
            response,
            mutation: MutationGate::Waived(waiver),
        }
    }

    /// Apply private-response headers and check every unsafe-method request.
    ///
    /// `render` runs synchronously for a rejected request inside the group's
    /// admission and private-response headers, and must not block the runtime.
    /// A request that passes the checks may present cookies through
    /// [`BrowserCredentials`](crate::browser::BrowserCredentials).
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
            mutation: MutationGate::Checked(Arc::new(MutationGuard {
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
    /// response headers, then admission, then the browser gate, which checks
    /// mutations and records whether the request may present credentials, then
    /// the application's own layers and handler.
    pub(super) fn apply(self, mut router: Router) -> Router {
        let Self { request, browser } = self;
        if let Some(browser) = &browser {
            router = router.layer(middleware::from_fn_with_state(
                browser.mutation.clone(),
                browser_gate,
            ));
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
async fn browser_gate(
    State(gate): State<MutationGate>,
    mut request: Request,
    next: Next,
) -> Response {
    // Decided from the method the request arrived with, before any application
    // layer can rewrite it.
    let access = if is_safe(request.method()) {
        CredentialAccess::Accepted
    } else {
        match &gate {
            MutationGate::Checked(guard) => match guard.policy.check(request.headers()) {
                Ok(()) => CredentialAccess::Accepted,
                Err(rejection) => {
                    let (parts, body) = request.into_parts();
                    drop(body);
                    return (guard.render)(rejection, &correlation::renderer_parts(parts));
                }
            },
            MutationGate::Waived(MutationCheckWaiver::ApplicationChecksMutations) => {
                CredentialAccess::Accepted
            }
            MutationGate::Waived(MutationCheckWaiver::NoAmbientCredentials) => {
                CredentialAccess::Refused
            }
        }
    };
    record_credential_access(request.extensions_mut(), access);
    next.run(request).await
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
///     response::IntoResponse,
///     routing::{get, post},
/// };
/// use batter_core::lifecycle::ShutdownHandle;
/// use batter_axum::{
///     AdmittedRequest, BrowserPolicy, GroupPolicy, GuardedRouter, HttpBoundary, ProbePath,
///     RequestPolicy, ResponseConstructionBudget, RouteGroup,
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
///     get(|| async { "profile" }).post(|admitted: AdmittedRequest| async move {
///         // The account group's own 10-second admission budget.
///         admitted.context().check().expect("admitted context");
///         "saved"
///     }),
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
/// # let _ = assembled.in_process();
/// # Ok(()) }
/// ```
///
/// A bare Axum router exposes no route inventory, so it cannot join a group
/// without [`GuardedRouter::from_router`](crate::GuardedRouter::from_router):
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
        if self.routes.route_patterns.is_empty() && self.routes.declared.is_empty() {
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
