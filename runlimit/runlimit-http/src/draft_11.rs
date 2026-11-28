//! Response fields from `draft-ietf-httpapi-ratelimit-headers-11`.
//!
//! This revision is dated 23 May 2026 and expired on 24 November 2026. The
//! current revision is [`draft_12`](crate::draft_12), which new code selects.
//! This module stays selectable, encoding exactly what it encoded before, so an
//! application whose clients parse `-11` fields can move them to `-12` as a
//! staged rollout rather than in one deploy. Its removal is a recorded
//! follow-up; the runlimit CHANGELOG announces it.
//!
//! The draft defines `RateLimit-Policy` for stable quota policy metadata and
//! `RateLimit` for the service limit currently available to a client. Both are
//! HTTP Structured Field lists whose members identify policies with String
//! items:
//!
//! | Field | Member | Parameters (`-11` sections) |
//! | --- | --- | --- |
//! | `RateLimit-Policy` | `"name";q=N;w=S` | `q` quota (3.1.1), `w` window (3.1.3) |
//! | `RateLimit` | `"name";r=N;t=S` | `r` available quota (4.1.1), `t` effective window (4.1.2) |
//!
//! `-12` renames `r` to `a` and reports only what a decision states exactly;
//! see [`draft_12`](crate::draft_12). Policy-name validation, the integer
//! range and [`EncodingError`] are shared with every draft module at the
//! crate root, and [`QuotaState`] is this module's `RateLimit` input.
//!
//! This module emits one list member at a time. Applications remain
//! responsible for selecting policies, combining multiple members when
//! appropriate, and deciding whether to attach the fields to a response.
//! Partition keys are intentionally unsupported so subject material cannot be
//! exposed accidentally.

use http::HeaderName;
use runlimit_core::RateLimitPolicy;

pub use crate::state::QuotaState;
use crate::{
    EncodingError, HeaderField,
    encoding::{self, Member},
};

/// Encodes one `-11` `RateLimit-Policy` field.
///
/// The result has the form `"name";q=N;w=S`, where `q` is
/// [`RateLimitPolicy::quota`] and `w` is its exact whole-second
/// [`RateLimitPolicy::quota_period`]. The default quota unit from the draft is
/// requests; this helper therefore omits the optional `qu` parameter.
///
/// # Errors
///
/// Returns [`EncodingError`] when the public policy name is unsafe for an
/// HTTP Structured Field String, a numeric value exceeds the Structured Field
/// integer range, or the quota period is not an exact whole number of
/// seconds.
pub fn quota_policy<P: RateLimitPolicy + ?Sized>(
    name: &str,
    policy: &P,
) -> Result<HeaderField, EncodingError> {
    encoding::quota_policy(name, policy)
}

/// Encodes one `-11` `RateLimit` field.
///
/// The result has the form `"name";r=N;t=S`. An available quota uses the
/// immediately available allowance and rounds its replenishment delay up to
/// whole seconds. An exhausted quota uses `r=0` and rounds the backend's retry
/// delay up in the same way. Neither creates a `Retry-After` field or chooses
/// an HTTP status.
///
/// # Errors
///
/// Returns [`EncodingError`] when the public policy name is unsafe or a
/// numeric value exceeds the Structured Field integer range.
pub fn service_limit(
    name: &str,
    state: impl Into<QuotaState>,
) -> Result<HeaderField, EncodingError> {
    // The name is validated before the caller's conversion runs, so an
    // unsafe name fails without converting a state whose field is never sent.
    let member = Member::policy(name)?;
    let limit = state.into().service_limit();
    Ok(member
        .integer("r", limit.available)?
        .integer("t", limit.effective_window)?
        .into_field(HeaderName::from_static("ratelimit")))
}
