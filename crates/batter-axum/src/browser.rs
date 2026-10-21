//! Browser-carried opaque credential transport.
//!
//! This module validates trusted origins, presents one named cookie without
//! duplicate ambiguity only where the route group's browser policy accepts
//! ambient credentials ([`BrowserCredentials`](crate::browser::BrowserCredentials)), emits fixed-scope cookies,
//! checks application-selected browser mutation signals, and applies
//! private-response headers with a typed referrer choice. It does not
//! authenticate callers, authorize requests, configure CORS, define a CSRF
//! token protocol, or clear server-side session state.
//!
//! All trusted configuration is constructed before serving. Request checks are
//! synchronous and do not log or retain observed header values.
//!
//! Migration: `read_cookie` is no longer exported here. Extract
//! [`BrowserCredentials`](crate::browser::BrowserCredentials) and call
//! [`BrowserCredentials::cookie`](crate::browser::BrowserCredentials::cookie); the
//! deliberately unchecked reader is
//! [`low_level::read_unchecked_cookie`](crate::low_level::read_unchecked_cookie).
//!
//! ```compile_fail,E0432
//! use batter_axum::browser::read_cookie;
//! ```

mod cookie_transport;
mod credentials;
mod mutation;
mod origin;
mod private_response;

pub(crate) use cookie_transport::read_fields;
pub use cookie_transport::{
    BrowserCookie, CookieHeaderError, CookieHeaderPolicy, CookieLifetime, CookieMaxAge, CookieName,
    CookieNameError, CookiePolicyError, CookieValue, CookieValueError, CookieVisibility, SameSite,
    SetCookieError,
};
pub use credentials::{BrowserCredentials, BrowserCredentialsRejection};
pub(crate) use credentials::{CredentialAccess, record as record_credential_access};
pub use mutation::{
    FetchSitePolicy, MutationPolicy, MutationPolicyError, MutationRejection, RequiredHeader,
    RequiredHeaderError,
};
pub use origin::{BrowserOrigin, BrowserOriginError};
pub use private_response::{
    PrivateResponseFuture, PrivateResponsePolicy, PrivateResponseService,
    apply_private_response_headers, private_response,
};
