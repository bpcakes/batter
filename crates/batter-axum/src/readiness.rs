use crate::HttpObservationLevel;
use axum::{
    extract::State,
    http::{StatusCode, request::Parts},
    response::Response,
};
use batter_core::{
    health::HealthReader,
    lifecycle::LifecycleStatus,
    readiness::{ReadinessCondition, ReadinessEvaluator, ReadinessUnreadyReason},
};
use std::convert::Infallible;
use tracing::Level;

pub use batter_core::readiness::ReadinessDecision;

/// Map a valid readiness decision to its default HTTP status.
///
/// This returns 200 only for [`ReadinessDecision::Ready`]; every unready
/// decision returns 503. Observation severity is independent.
///
/// ```
/// use axum::http::StatusCode;
/// use batter_core::readiness::{ReadinessDecision, ReadinessUnreadyReason};
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
/// Ready, Starting and Draining use INFO. Stopped, dependency-unready and
/// unsatisfied application-condition decisions use WARN. A custom
/// [`ReadinessPolicy::with_level`] callback can delegate unmatched decisions
/// here instead of copying the default table.
///
/// ```
/// use batter_core::readiness::{ReadinessDecision, ReadinessUnreadyReason};
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
            ReadinessUnreadyReason::Stopped
            | ReadinessUnreadyReason::Dependency(_)
            | ReadinessUnreadyReason::Condition(_),
        ) => Level::WARN,
    }
}

/// Read-only dependency, lifecycle and application-condition readiness with
/// explicit severity policy.
///
/// Reads never invoke the probe, refresh a timestamp or retain writer ownership.
/// Lifecycle is checked after dependency sampling and any application
/// conditions, so an observed drain overrides cached success. This is a
/// point-in-time decision, not atomic with later drain. Mount outside
/// admission, as [`crate::HttpBoundary::with_readiness`] and
/// [`crate::HttpBoundary::with_rendered_readiness`] do. Existing
/// [`crate::low_level::readiness`] remains status-only.
///
/// ```
/// use axum::{Router, routing::get};
/// use batter_core::{health::HealthReader, lifecycle::ShutdownHandle};
/// use batter_axum::{ReadinessPolicy, low_level::dependency_readiness};
/// fn probes(control: ShutdownHandle, health: HealthReader<std::io::Error>) -> Router {
///     Router::new().route("/ready", get(dependency_readiness::<std::io::Error>))
///         .with_state(ReadinessPolicy::new(control.status(), health))
/// }
/// ```
///
/// Root shutdown control cannot be retained by readiness policy:
///
/// ```compile_fail,E0308
/// use batter_core::{health::HealthReader, lifecycle::ShutdownHandle};
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
/// use batter_core::readiness::ReadinessReason;
/// ```
#[must_use = "retain the configured policy; a discarded result keeps the previous severity"]
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

impl ReadinessPolicy<Infallible> {
    /// Use lifecycle and application conditions without creating a dependency monitor.
    ///
    /// This asserts no continuous dependency observation, not healthy remote
    /// connectivity. Conditions run before the final lifecycle read and can
    /// only narrow readiness. The ordinary status and severity mappings apply.
    ///
    /// ```
    /// use batter_axum::ReadinessPolicy;
    /// use batter_core::{lifecycle::ShutdownHandle, readiness::ReadinessCondition};
    /// let (control, approval) = ShutdownHandle::new_with_readiness_approval();
    /// let policy = ReadinessPolicy::lifecycle_only(control.status())
    ///     .with_condition(ReadinessCondition::new("application-state")?, || true);
    /// approval.approve();
    /// assert!(policy.decision().is_ready());
    /// # Ok::<(), batter_core::readiness::ReadinessConditionError>(())
    /// ```
    pub fn lifecycle_only(lifecycle: LifecycleStatus) -> Self {
        Self {
            evaluator: ReadinessEvaluator::lifecycle_only(lifecycle),
            level: default_readiness_level,
        }
    }
}

impl<E> ReadinessPolicy<E> {
    /// Select INFO for expected Starting/Draining, WARN for dependency,
    /// application-condition and Stopped failures.
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

    /// Narrow readiness with an application condition.
    ///
    /// While `satisfied` returns `false`, a decision that would otherwise be
    /// Ready is `Unready(ReadinessUnreadyReason::Condition(condition))`, which
    /// renders 503 and defaults to WARN. The condition is asked only when the
    /// dependency is ready or absent and cannot make an unready lifecycle or dependency
    /// ready; see [`ReadinessEvaluator::with_condition`] for when it runs.
    /// Conditions accumulate in the order added. Condition names are validated
    /// [`ReadinessCondition`]s, never raw strings:
    ///
    /// ```compile_fail,E0308
    /// use batter_axum::ReadinessPolicy;
    ///
    /// fn unvalidated(policy: ReadinessPolicy<std::io::Error>) {
    ///     let _ = policy.with_condition("key-leases", || true);
    /// }
    /// ```
    pub fn with_condition<F>(mut self, condition: ReadinessCondition, satisfied: F) -> Self
    where
        F: Fn() -> bool + Send + Sync + 'static,
    {
        self.evaluator = self.evaluator.with_condition(condition, satisfied);
        self
    }

    /// Obtain a fresh read-only decision. No dependency cause enters the response.
    pub fn decision(&self) -> ReadinessDecision {
        self.evaluator.decision()
    }

    /// Empty-body 200/503 with typed decision and severity response extensions.
    pub fn response(&self) -> Response {
        let decision = self.decision();
        self.finish(decision, Response::default())
    }

    /// Render a fresh decision with the application's renderer, then apply the
    /// adapter-owned status, decision extension and severity.
    pub(crate) fn render(&self, parts: &Parts, renderer: &ReadinessRenderer) -> Response {
        let decision = self.decision();
        self.finish(decision, renderer(decision, parts))
    }

    /// Replace the status and the decision and severity extensions, whatever a
    /// renderer set, so only the decision and the severity policy select them.
    fn finish(&self, decision: ReadinessDecision, mut response: Response) -> Response {
        *response.status_mut() = readiness_status(decision);
        let extensions = response.extensions_mut();
        extensions.insert(decision);
        extensions.insert(HttpObservationLevel((self.level)(decision)));
        response
    }
}

/// An application renderer for readiness probe responses.
pub(crate) type ReadinessRenderer = dyn Fn(ReadinessDecision, &Parts) -> Response + Send + Sync;

/// Serve [`ReadinessPolicy`]'s read-only decision outside guarded routes.
///
/// Migration: `batter_axum::dependency_readiness` moved to
/// `batter_axum::low_level::dependency_readiness` with the same caller obligations.
pub async fn dependency_readiness<E>(State(policy): State<ReadinessPolicy<E>>) -> Response {
    policy.response()
}
