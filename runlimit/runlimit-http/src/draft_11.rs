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
//! Only the `RateLimit` keys differ from `-12`, which spells them `a` and `w`.
//! Policy-name validation, the integer range, [`QuotaState`] and
//! [`EncodingError`] are shared with every draft module at the crate root.
//!
//! This module emits one list member at a time. Applications remain
//! responsible for selecting policies, combining multiple members when
//! appropriate, and deciding whether to attach the fields to a response.
//! Partition keys are intentionally unsupported so subject material cannot be
//! exposed accidentally.

use http::HeaderName;
use runlimit_core::RateLimitPolicy;

use crate::{
    EncodingError, HeaderField, QuotaState,
    encoding::{self, Member, ServiceLimitKeys, policy_window},
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
    let member = Member::policy(name)?
        .integer("q", policy.quota().get())?
        .integer("w", policy_window(policy.quota_period())?)?;
    Ok(member.into_field(HeaderName::from_static("ratelimit-policy")))
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
    encoding::service_limit(
        name,
        state,
        &ServiceLimitKeys {
            available: "r",
            window: "t",
        },
    )
}
