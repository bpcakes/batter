use crate::{ReadinessPolicy, quota_observation::QuotaObservation, readiness::ReadinessRenderer};
use axum::{
    extract::Request,
    http::{StatusCode, request::Parts},
    response::Response,
    routing::{MethodRouter, get},
};
use std::{error::Error, fmt, sync::Arc};

/// An application renderer for liveness probe responses.
type LivenessRenderer = dyn Fn(&Parts) -> Response + Send + Sync;

/// Serve liveness with the application's response and the fixed 200 status.
pub(super) fn rendered_liveness(render: Arc<LivenessRenderer>) -> MethodRouter {
    get(move |request: Request| {
        let render = render.clone();
        async move {
            let mut response = render(&renderer_parts(request));
            // Answering at all is the liveness signal; no renderer can change it.
            *response.status_mut() = StatusCode::OK;
            response
        }
    })
}

/// Serve one fresh readiness decision per request with the application's
/// response; the policy then applies the decision's status and extensions.
pub(super) fn rendered_readiness<E>(
    policy: ReadinessPolicy<E>,
    render: Arc<ReadinessRenderer>,
) -> MethodRouter
where
    E: Send + Sync + 'static,
{
    get(move |request: Request| {
        let (policy, render) = (policy.clone(), render.clone());
        async move { policy.render(&renderer_parts(request), render.as_ref()) }
    })
}

/// The probe request's metadata without its body or the observer's private
/// quota writer.
fn renderer_parts(request: Request) -> Parts {
    let (mut parts, body) = request.into_parts();
    drop(body);
    parts.extensions.remove::<QuotaObservation>();
    parts
}

/// A validated, literal route for a process probe.
///
/// Probe paths deliberately accept only absolute ASCII paths made from
/// unreserved URI characters. Axum capture, wildcard and legacy pattern syntax
/// cannot be represented, so a probe cannot become a public fallback for
/// guarded application routes.
///
/// ```
/// use batter_axum::ProbePath;
///
/// let path = ProbePath::new("/health/live")?;
/// assert_eq!(path.as_str(), "/health/live");
/// # Ok::<(), batter_axum::ProbePathError>(())
/// ```
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[must_use = "retain the validated probe path"]
pub struct ProbePath(&'static str);

impl ProbePath {
    /// Validate one absolute, literal probe path.
    ///
    /// Each non-empty segment may contain only ASCII letters, digits, `-`,
    /// `.`, `_`, or `~`. Empty, dot, capture, wildcard, query, fragment and
    /// percent-encoded segments are rejected. `/` is accepted as the root
    /// literal. The input is retained only after validation and is never
    /// included in the sanitized error.
    pub fn new(path: &'static str) -> Result<Self, ProbePathError> {
        validate_probe_path(path)?;
        Ok(Self(path))
    }

    /// Return the exact validated route handed to Axum.
    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

fn validate_probe_path(path: &str) -> Result<(), ProbePathError> {
    if !path.starts_with('/') {
        return Err(ProbePathError::NotAbsolute);
    }
    if path == "/" {
        return Ok(());
    }
    if path.len() > 256 {
        return Err(ProbePathError::TooLong);
    }
    for segment in path[1..].split('/') {
        if segment.is_empty() {
            return Err(ProbePathError::EmptySegment);
        }
        if segment == "." || segment == ".." {
            return Err(ProbePathError::DotSegment);
        }
        if segment
            .bytes()
            .any(|byte| !byte.is_ascii_alphanumeric() && !b"-._~".contains(&byte))
        {
            return Err(ProbePathError::NonLiteralSegment);
        }
    }
    Ok(())
}

/// Sanitized failure to construct a literal [`ProbePath`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ProbePathError {
    /// The path is empty or does not begin with `/`.
    NotAbsolute,
    /// The path is longer than the boundary's fixed configuration limit.
    TooLong,
    /// The path contains a repeated separator or trailing slash.
    EmptySegment,
    /// The path contains `.` or `..` as a complete segment.
    DotSegment,
    /// A segment contains route syntax or a non-unreserved character.
    NonLiteralSegment,
}

impl fmt::Display for ProbePathError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NotAbsolute => "probe path must be absolute",
            Self::TooLong => "probe path is too long",
            Self::EmptySegment => "probe path contains an empty segment",
            Self::DotSegment => "probe path contains a dot segment",
            Self::NonLiteralSegment => "probe path contains non-literal route syntax",
        })
    }
}

impl Error for ProbePathError {}

/// Sanitized failure to register a probe in an [`HttpBoundary`](crate::HttpBoundary).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ProbeRegistrationError {
    /// The same literal path was already assigned to another probe.
    DuplicatePath,
}

impl fmt::Display for ProbeRegistrationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::DuplicatePath => "probe path is already registered",
        })
    }
}

impl Error for ProbeRegistrationError {}
