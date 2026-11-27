//! Response fields from `draft-ietf-httpapi-ratelimit-headers-12`.
//!
//! This is the current revision, dated 20 November 2026. It is still an
//! Internet-Draft, not a published RFC, so a later revision can change this
//! wire format again; that revision would get its own module.
//!
//! `RateLimit-Policy` advertises a stable quota policy and `RateLimit` the
//! service limit currently available to a client. Both are HTTP Structured
//! Field lists whose members identify policies; this module always emits the
//! identifier as a String item, which `-12` accepts beside a Token:
//!
//! | Field | Member | Parameters (`-12` sections) |
//! | --- | --- | --- |
//! | `RateLimit-Policy` | `"name";q=N;w=S` | `q` quota (2.1.1), `w` window (2.1.3) |
//! | `RateLimit` | `"name";a=N;w=S` | `a` available quota (3.1.1), `w` effective window (3.1.2) |
//!
//! `RateLimit-Policy` is spelled as in `-11`; `RateLimit` renames `-11`'s `r`
//! and `t` to `a` and `w`. Policy-name validation, the integer range,
//! [`QuotaState`] and [`EncodingError`] are shared with every draft module at
//! the crate root, so an input `-11` rejects fails here with the same error.
//!
//! These optional parts of `-12` are not emitted:
//!
//! - `qu` (2.1.2): omitted, so a client reads the draft's default unit,
//!   requests, as `-11` encoding did. A policy whose checks charge another
//!   unit cannot state it through this module.
//! - `pk` (2.1.4, 3.1.3): partition keys are unsupported so subject material
//!   cannot be exposed accidentally.
//! - `c` (3.1.4), the request's cost in quota units: a [`QuotaState`] does not
//!   carry the cost of the check it came from, and a separately supplied
//!   number could describe a different check, so this module states no cost.
//! - The `RateLimit-Partition` field (section 4). Its items declare a
//!   policy's partition dimensions, which build `pk` when both fields are
//!   sent (4.2), and can limit a policy to matching requests, such as
//!   `method=GET`, so a client can tell whether it applies (4.1). A Runlimit
//!   subject is an opaque derived key and a policy does not record which
//!   requests the application routes to it, so neither declaration is known
//!   here; an application that knows them can send the field itself.
//!
//! This module emits one list member at a time. Applications remain
//! responsible for selecting policies, combining members, and deciding which
//! responses carry the fields; `-12` section 5 advises against sending them on
//! redirections. `-12` section 2.1 compares policy identifiers
//! case-insensitively and forbids two equivalent identifiers in one field, and
//! field lines with the same name combine into one field, so names an
//! application sends together must differ by more than ASCII case.

use runlimit_core::RateLimitPolicy;

use crate::{
    EncodingError, HeaderField, QuotaState,
    encoding::{self, ServiceLimitKeys},
};

/// Encodes one `-12` `RateLimit-Policy` field.
///
/// The result has the form `"name";q=N;w=S`, where `q` is
/// [`RateLimitPolicy::quota`] and `w` is its exact whole-second
/// [`RateLimitPolicy::quota_period`].
///
/// ```
/// use runlimit_core::{FixedWindowPolicy, PolicyId, ScopeId};
/// use std::time::Duration;
///
/// # fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let policy = FixedWindowPolicy::new(
///     PolicyId::new("example.read")?,
///     ScopeId::new("owner")?,
///     100,
///     Duration::from_secs(60),
/// )?;
/// let (name, value) = runlimit_http::draft_12::quota_policy("reads", &policy)?;
/// assert_eq!(name.as_str(), "ratelimit-policy");
/// assert_eq!(value.to_str()?, "\"reads\";q=100;w=60");
/// # Ok(())
/// # }
/// # example().unwrap();
/// ```
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

/// Encodes one `-12` `RateLimit` field.
///
/// The result has the form `"name";a=N;w=S`. An available quota uses the
/// immediately available allowance and rounds its replenishment delay up to
/// whole seconds. An exhausted quota uses `a=0` and rounds the backend's retry
/// delay up in the same way, so `w` never understates the delay the backend
/// reported at its evaluation instant. It is a snapshot, not a promise of
/// admission: another request sharing the quota can spend the replenished
/// units first. Neither creates a `Retry-After` field or chooses an HTTP
/// status.
///
/// ```
/// use runlimit_core::{Allowance, Capacity};
/// use std::time::Duration;
///
/// # fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let allowance = Allowance::new(Capacity::new(100)?, 99, Duration::from_millis(49_001))?;
/// let (name, value) = runlimit_http::draft_12::service_limit("reads", allowance)?;
/// assert_eq!(name.as_str(), "ratelimit");
/// assert_eq!(value.to_str()?, "\"reads\";a=99;w=50");
/// # Ok(())
/// # }
/// # example().unwrap();
/// ```
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
            available: "a",
            window: "w",
        },
    )
}
