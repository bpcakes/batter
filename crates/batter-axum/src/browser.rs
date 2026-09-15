//! Browser-carried opaque credential transport.
//!
//! This module validates trusted origins, reads one named cookie without
//! duplicate ambiguity, emits fixed-scope cookies, checks application-selected
//! browser mutation signals, and applies private-response headers. It does not
//! authenticate callers, authorize requests, configure CORS, define a CSRF
//! token protocol, or clear server-side session state.
//!
//! All trusted configuration is constructed before serving. Request checks are
//! synchronous and do not log or retain observed header values.

mod cookie_transport;
mod mutation;
mod origin;
mod private_response;

pub use cookie_transport::{
    BrowserCookie, CookieHeaderError, CookieHeaderPolicy, CookieLifetime, CookieMaxAge, CookieName,
    CookieNameError, CookiePolicyError, CookieValue, CookieValueError, SameSite, SetCookieError,
};
pub use cookie_transport::{CookieVisibility, read_cookie};
pub use mutation::{
    FetchSitePolicy, MutationPolicy, MutationPolicyError, MutationRejection, RequiredHeader,
    RequiredHeaderError,
};
pub use origin::{BrowserOrigin, BrowserOriginError};
pub use private_response::{apply_private_response_headers, private_response};
