use crate::HttpObservationLevel;
use axum::{
    Extension,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use batter::{
    health::{HealthReader, HealthStatus},
    lifecycle::{Readiness, ShutdownHandle},
};
use tracing::Level;

/// Sanitized reason for a point-in-time readiness decision, with no cause data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadinessReason {
    /// Lifecycle Ready and a fresh healthy dependency observation.
    Ready,
    /// Application approval, driver or component acknowledgement is outstanding.
    Starting,
    /// Admission has closed and shutdown is underway.
    Draining,
    /// The process has stopped; distinct from expected startup/drain.
    Stopped,
    /// Lifecycle Ready, but the dependency is not healthy (including writer loss).
    Dependency(HealthStatus),
}

impl ReadinessReason {
    /// 200 only for Ready, otherwise 503; independent of observation severity.
    pub const fn status(self) -> StatusCode {
        match self {
            Self::Ready => StatusCode::OK,
            _ => StatusCode::SERVICE_UNAVAILABLE,
        }
    }

    /// INFO for Ready/Starting/Draining; WARN for Stopped or dependency failures.
    pub const fn level(self) -> Level {
        match self {
            Self::Ready | Self::Starting | Self::Draining => Level::INFO,
            Self::Stopped | Self::Dependency(_) => Level::WARN,
        }
    }
}

/// Read-only dependency and lifecycle readiness with explicit severity policy.
///
/// Reads never invoke the probe, refresh a timestamp or retain writer ownership.
/// Lifecycle is checked after dependency sampling so an observed drain overrides
/// cached success. This is a point-in-time decision, not atomic with later drain.
/// Mount outside admission. Existing [`crate::readiness`] remains status-only.
///
/// ```
/// use axum::{Router, routing::get};
/// use batter::{health::HealthReader, lifecycle::ShutdownHandle};
/// use batter_axum::{ReadinessPolicy, dependency_readiness};
/// fn probes(handle: ShutdownHandle, health: HealthReader<std::io::Error>) -> Router {
///     Router::new().route("/ready", get(dependency_readiness::<std::io::Error>))
///         .with_state(ReadinessPolicy::new(handle, health))
/// }
/// ```
pub struct ReadinessPolicy<E> {
    lifecycle: ShutdownHandle,
    dependency: HealthReader<E>,
    level: fn(ReadinessReason) -> Level,
}

impl<E> Clone for ReadinessPolicy<E> {
    fn clone(&self) -> Self {
        Self {
            lifecycle: self.lifecycle.clone(),
            dependency: self.dependency.clone(),
            level: self.level,
        }
    }
}

impl<E> ReadinessPolicy<E> {
    /// Select INFO for expected Starting/Draining, WARN for dependency/Stopped failures.
    pub fn new(lifecycle: ShutdownHandle, dependency: HealthReader<E>) -> Self {
        Self {
            lifecycle,
            dependency,
            level: ReadinessReason::level,
        }
    }

    /// Explicitly override completion severity without changing status or body.
    /// The callback receives only the sanitized reason and runs during rendering,
    /// never in a destructor. Subscriber filters still determine event delivery.
    pub fn with_level(mut self, level: fn(ReadinessReason) -> Level) -> Self {
        self.level = level;
        self
    }

    /// Obtain a fresh read-only reason. No dependency cause enters the response.
    pub fn reason(&self) -> ReadinessReason {
        let health = self.dependency.snapshot().status();
        match self.lifecycle.readiness() {
            Readiness::Starting => ReadinessReason::Starting,
            Readiness::Draining => ReadinessReason::Draining,
            Readiness::Stopped => ReadinessReason::Stopped,
            Readiness::Ready if health == HealthStatus::Healthy => ReadinessReason::Ready,
            Readiness::Ready => ReadinessReason::Dependency(health),
        }
    }

    /// Empty-body 200/503 with typed reason and severity response extensions.
    pub fn response(&self) -> Response {
        let reason = self.reason();
        (
            Extension(reason),
            Extension(HttpObservationLevel((self.level)(reason))),
            reason.status(),
        )
            .into_response()
    }
}

/// Serve [`ReadinessPolicy`]'s read-only decision outside guarded routes.
pub async fn dependency_readiness<E>(State(policy): State<ReadinessPolicy<E>>) -> Response {
    policy.response()
}
