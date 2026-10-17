//! The cookies a route group's browser policy lets one request present.

use super::{
    CookieHeaderError, CookieHeaderPolicy, CookieName, CookieValue, cookie_transport::read_fields,
};
use crate::HttpFailure;
use axum::{
    extract::FromRequestParts,
    http::{Extensions, HeaderValue, header, request::Parts},
    response::{IntoResponse, Response},
};
use std::{error::Error, fmt};

/// What a browser group's policy decided for one request.
///
/// Only this crate can name the type, so no application layer can insert,
/// replace or remove the record by type. The group's gate decides from the
/// method the request arrived with, before any application layer runs.
#[derive(Clone, Copy, Debug)]
pub(crate) enum CredentialAccess {
    /// A safe method, a request that passed the group's mutation checks, or a
    /// mutation the application declared it verifies itself.
    Accepted,
    /// A mutation on a group that waived its checks because its mutating
    /// routes accept no ambient credentials.
    Refused,
}

/// Record the group's decision, replacing any record the request carries.
pub(crate) fn record(extensions: &mut Extensions, access: CredentialAccess) {
    extensions.insert(access);
}

/// The Cookie fields of a request whose route group's
/// [`BrowserPolicy`](crate::BrowserPolicy) accepts ambient browser credentials.
///
/// This is the canonical way to read a browser cookie. Extract it in a handler
/// or route layer of an [`HttpBoundary`](crate::HttpBoundary) group and read
/// one named cookie with [`Self::cookie`]. The group's browser policy decides,
/// from the method the request arrived with and before any application layer
/// runs, whether the request may present cookies:
///
/// | Group policy | `GET`, `HEAD`, `OPTIONS`, `TRACE` | Every other method |
/// | --- | --- | --- |
/// | [`BrowserPolicy::with_mutation_checks`](crate::BrowserPolicy::with_mutation_checks) | presented | presented after the checks pass; a failed check never reaches the handler |
/// | `without_mutation_checks(_, `[`ApplicationChecksMutations`](crate::MutationCheckWaiver::ApplicationChecksMutations)`)` | presented | presented; the application verifies the request |
/// | `without_mutation_checks(_, `[`NoAmbientCredentials`](crate::MutationCheckWaiver::NoAmbientCredentials)`)` | presented | [`BrowserCredentialsRejection::UncheckedMutation`] |
/// | no browser policy, or outside every group | [`BrowserCredentialsRejection::NoBrowserPolicy`] | [`BrowserCredentialsRejection::NoBrowserPolicy`] |
///
/// The decision does not depend on whether the request carries a Cookie
/// field, so a handler placed in the wrong group fails on its first request of
/// that method instead of only when a browser sends a cookie. A layer that
/// rewrites the method cannot change it.
///
/// ```
/// use axum::{
///     body::Body,
///     extract::Request,
///     http::{Method, StatusCode, header},
///     response::IntoResponse,
///     routing::post,
/// };
/// use batter_axum::{
///     BrowserPolicy, GroupPolicy, GuardedRouter, HttpBoundary, RequestPolicy,
///     ResponseConstructionBudget, RouteGroup,
///     browser::{
///         BrowserCredentials, BrowserOrigin, CookieHeaderPolicy, CookieName, MutationPolicy,
///         PrivateResponsePolicy,
///     },
/// };
/// use batter_core::lifecycle::ShutdownHandle;
/// use std::time::Duration;
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let (control, approval) = ShutdownHandle::new_with_readiness_approval();
/// approval.approve();
/// let budget = ResponseConstructionBudget::new(Duration::from_secs(5))?;
/// let request_policy = || RequestPolicy::new(control.operation_admission(), budget);
///
/// async fn save(credentials: BrowserCredentials) -> impl IntoResponse {
///     let name = CookieName::new("__Host-session").expect("valid name");
///     match credentials.cookie(&name, CookieHeaderPolicy::TargetOnly) {
///         // Hand the opaque value to the application's session lookup.
///         Ok(Some(_session)) => StatusCode::NO_CONTENT,
///         Ok(None) => StatusCode::UNAUTHORIZED,
///         Err(_) => StatusCode::BAD_REQUEST,
///     }
/// }
///
/// let browser = BrowserPolicy::with_mutation_checks(
///     PrivateResponsePolicy::SameOriginReferrer,
///     MutationPolicy::exact_origin(BrowserOrigin::https("https://app.example")?),
///     |rejection, _parts| (rejection.status(), rejection.code()).into_response(),
/// );
/// let client = HttpBoundary::new(request_policy())
///     .with_group(RouteGroup::new(
///         "account",
///         GroupPolicy::browser(request_policy(), browser),
///         GuardedRouter::new().route("/account", post(save)),
///     ))?
///     .assemble(GuardedRouter::new())
///     .await?
///     .in_process();
///
/// let saved = Request::builder()
///     .method(Method::POST)
///     .uri("/account")
///     .header(header::ORIGIN, "https://app.example")
///     .header(header::COOKIE, "__Host-session=opaque")
///     .body(Body::empty())?;
/// assert_eq!(client.request(saved).await.status(), StatusCode::NO_CONTENT);
///
/// let forged = Request::builder()
///     .method(Method::POST)
///     .uri("/account")
///     .header(header::ORIGIN, "https://attacker.example")
///     .header(header::COOKIE, "__Host-session=opaque")
///     .body(Body::empty())?;
/// assert_eq!(client.request(forged).await.status(), StatusCode::FORBIDDEN);
/// # Ok(()) }
/// ```
///
/// Only the extractor creates the value. Its field is private and it has no
/// constructor, so request headers cannot become credentials without the
/// group's decision:
///
/// ```compile_fail,E0451
/// use batter_axum::browser::BrowserCredentials;
///
/// fn forge() -> BrowserCredentials {
///     BrowserCredentials { cookies: Box::new([]) }
/// }
/// ```
///
/// The group's gate records its decision privately; inserting this value into
/// request extensions would not change it, and the value is not `Clone`, so
/// it cannot be inserted:
///
/// ```compile_fail,E0277
/// use axum::extract::Request;
/// use batter_axum::browser::BrowserCredentials;
///
/// fn transplant(credentials: BrowserCredentials, mut request: Request) {
///     request.extensions_mut().insert(credentials);
/// }
/// ```
///
/// Copying a whole extension map from one request into another carries the
/// group's record too, like every other value in the map. A handler can still
/// parse the raw `Cookie` field itself, and
/// [`low_level::read_unchecked_cookie`](crate::low_level::read_unchecked_cookie)
/// reads one without the group's decision; both leave the mutation check with
/// the caller.
pub struct BrowserCredentials {
    cookies: Box<[HeaderValue]>,
}

impl BrowserCredentials {
    /// Read one exact named cookie across every Cookie field the request
    /// carried.
    ///
    /// Duplicate target cookies, non-UTF8 fields, and malformed target values
    /// are always rejected. `TargetOnly` relaxes only unrelated malformed
    /// pairs. An empty target value is returned as present and unchanged; the
    /// application credential parser decides whether that value is meaningful.
    ///
    /// Migration: the removed `batter_axum::browser::read_cookie(&headers, &name,
    /// policy)` took any header map. Extract `BrowserCredentials` in the handler
    /// instead and call `credentials.cookie(&name, policy)`, with the same
    /// policy, results and errors; put the route in a group whose
    /// [`BrowserPolicy`](crate::BrowserPolicy) accepts credentials for its
    /// methods.
    pub fn cookie(
        &self,
        name: &CookieName,
        policy: CookieHeaderPolicy,
    ) -> Result<Option<CookieValue<'_>>, CookieHeaderError> {
        read_fields(self.cookies.iter(), name, policy)
    }
}

impl fmt::Debug for BrowserCredentials {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BrowserCredentials")
            .field("cookie_fields", &self.cookies.len())
            .finish_non_exhaustive()
    }
}

impl<S: Send + Sync> FromRequestParts<S> for BrowserCredentials {
    type Rejection = BrowserCredentialsRejection;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        match parts.extensions.get::<CredentialAccess>() {
            Some(CredentialAccess::Accepted) => Ok(Self {
                cookies: parts
                    .headers
                    .get_all(header::COOKIE)
                    .iter()
                    .cloned()
                    .collect(),
            }),
            Some(CredentialAccess::Refused) => Err(BrowserCredentialsRejection::UncheckedMutation),
            None => Err(BrowserCredentialsRejection::NoBrowserPolicy),
        }
    }
}

/// Sanitized refusal to present browser credentials for one request.
///
/// Both variants are composition errors: the handler that reads cookies sits
/// where the route group's policy does not accept them. Each answers exactly as
/// [`HttpFailure::Internal`] does: status 500 with the fixed
/// `application/problem+json` body and `Cache-Control: no-store`, naming no
/// cookie, route or request field, at the default WARN completion severity.
/// To use the application's own envelope instead, extract
/// `Result<BrowserCredentials, BrowserCredentialsRejection>` and render the
/// error.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum BrowserCredentialsRejection {
    /// The request reached no route group with a
    /// [`BrowserPolicy`](crate::BrowserPolicy): a group built without one, a
    /// lower-level composition or a router outside the boundary.
    NoBrowserPolicy,
    /// A method other than `GET`, `HEAD`, `OPTIONS` or `TRACE` reached a group
    /// whose policy waived mutation checks with
    /// [`MutationCheckWaiver::NoAmbientCredentials`](crate::MutationCheckWaiver::NoAmbientCredentials).
    UncheckedMutation,
}

impl fmt::Display for BrowserCredentialsRejection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NoBrowserPolicy => "request reached no route group with a browser policy",
            Self::UncheckedMutation => {
                "route group accepts no ambient browser credentials on unchecked mutations"
            }
        })
    }
}

impl Error for BrowserCredentialsRejection {}

impl IntoResponse for BrowserCredentialsRejection {
    fn into_response(self) -> Response {
        HttpFailure::Internal.into_response()
    }
}
