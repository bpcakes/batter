use crate::HttpObservationLevel;
use axum::{
    Extension,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use batter::{
    health::HealthReader,
    lifecycle::LifecycleStatus,
    readiness::{ReadinessEvaluator, ReadinessUnreadyReason},
};
use tracing::Level;

pub use batter::readiness::ReadinessDecision;

/// Map a valid readiness decision to its default HTTP status.
///
/// This returns 200 only for [`ReadinessDecision::Ready`]; every unready
/// decision returns 503. Observation severity is independent.
///
/// ```
/// use axum::http::StatusCode;
/// use batter::readiness::{ReadinessDecision, ReadinessUnreadyReason};
/// use batter_axum::readiness_status;
///
/// assert_eq!(readiness_status(ReadinessDecision::Ready), StatusCode::OK);
/// assert_eq!(
///     readiness_status(ReadinessDecision::Unready(ReadinessUnreadyReason::Starting)),
///     StatusCode::SERVICE_UNAVAILABLE,
/// );
/// ```
pub const fn readiness_status(decision: ReadinessDecision) -> StatusCode {
    match decision {
        ReadinessDecision::Ready => StatusCode::OK,
        ReadinessDecision::Unready(_) => StatusCode::SERVICE_UNAVAILABLE,
    }
}

/// Map a valid readiness decision to its default observation severity.
///
/// Ready, Starting and Draining use INFO. Stopped and dependency-unready
/// decisions use WARN. A custom [`ReadinessPolicy::with_level`] callback can
/// delegate unmatched decisions here instead of copying the default table.
///
/// ```
/// use batter::readiness::{ReadinessDecision, ReadinessUnreadyReason};
/// use batter_axum::default_readiness_level;
/// use tracing::Level;
///
/// fn starting_is_debug(decision: ReadinessDecision) -> Level {
///     match decision {
///         ReadinessDecision::Unready(ReadinessUnreadyReason::Starting) => Level::DEBUG,
///         other => default_readiness_level(other),
///     }
/// }
///
/// assert_eq!(
///     starting_is_debug(ReadinessDecision::Unready(ReadinessUnreadyReason::Starting)),
///     Level::DEBUG,
/// );
/// assert_eq!(
///     starting_is_debug(ReadinessDecision::Unready(ReadinessUnreadyReason::Stopped)),
///     Level::WARN,
/// );
/// ```
pub const fn default_readiness_level(decision: ReadinessDecision) -> Level {
    match decision {
        ReadinessDecision::Ready
        | ReadinessDecision::Unready(
            ReadinessUnreadyReason::Starting | ReadinessUnreadyReason::Draining,
        ) => Level::INFO,
        ReadinessDecision::Unready(
            ReadinessUnreadyReason::Stopped | ReadinessUnreadyReason::Dependency(_),
        ) => Level::WARN,
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
/// fn probes(control: ShutdownHandle, health: HealthReader<std::io::Error>) -> Router {
///     Router::new().route("/ready", get(dependency_readiness::<std::io::Error>))
///         .with_state(ReadinessPolicy::new(control.status(), health))
/// }
/// ```
///
/// Root shutdown control cannot be retained by readiness policy:
///
/// ```compile_fail,E0308
/// use batter::{health::HealthReader, lifecycle::ShutdownHandle};
/// use batter_axum::ReadinessPolicy;
///
/// fn cannot_retain_control(control: ShutdownHandle, health: HealthReader<std::io::Error>) {
///     let policy = ReadinessPolicy::new(control, health);
/// }
/// ```
///
/// The pre-cutover `ReadinessReason` name is intentionally unavailable from
/// both the adapter and foundation, so changing only an import cannot make an
/// old typed extension lookup compile and silently return `None`. Match the
/// explicitly unready-only [`ReadinessUnreadyReason`] inside the decision:
///
/// ```compile_fail,E0432
/// use batter_axum::ReadinessReason;
/// ```
///
/// ```compile_fail,E0432
/// use batter::readiness::ReadinessReason;
/// ```
pub struct ReadinessPolicy<E> {
    evaluator: ReadinessEvaluator<E>,
    level: fn(ReadinessDecision) -> Level,
}

impl<E> Clone for ReadinessPolicy<E> {
    fn clone(&self) -> Self {
        Self {
            evaluator: self.evaluator.clone(),
            level: self.level,
        }
    }
}

impl<E> ReadinessPolicy<E> {
    /// Select INFO for expected Starting/Draining, WARN for dependency/Stopped failures.
    pub fn new(lifecycle: LifecycleStatus, dependency: HealthReader<E>) -> Self {
        Self {
            evaluator: ReadinessEvaluator::new(lifecycle, dependency),
            level: default_readiness_level,
        }
    }

    /// Explicitly override completion severity without changing status or body.
    /// The callback receives only the sanitized decision and runs during
    /// rendering, never in a destructor. Subscriber filters still determine
    /// event delivery.
    pub fn with_level(mut self, level: fn(ReadinessDecision) -> Level) -> Self {
        self.level = level;
        self
    }

    /// Obtain a fresh read-only decision. No dependency cause enters the response.
    pub fn decision(&self) -> ReadinessDecision {
        self.evaluator.decision()
    }

    /// Empty-body 200/503 with typed decision and severity response extensions.
    pub fn response(&self) -> Response {
        let decision = self.decision();
        (
            Extension(decision),
            Extension(HttpObservationLevel((self.level)(decision))),
            readiness_status(decision),
        )
            .into_response()
    }
}

/// Serve [`ReadinessPolicy`]'s read-only decision outside guarded routes.
pub async fn dependency_readiness<E>(State(policy): State<ReadinessPolicy<E>>) -> Response {
    policy.response()
}
