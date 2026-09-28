//! Foundation-owned process readiness over lifecycle, dependency and
//! application-condition observations.
//!
//! The evaluator performs no probe I/O. It samples dependency health first,
//! then any application conditions, and lifecycle last, so a lifecycle
//! transition observed during the decision overrides an earlier dependency or
//! condition result. Application conditions can only narrow readiness. HTTP
//! status and telemetry severity remain adapter policy.

use crate::{
    health::{DependencyReadiness, DependencyUnreadyReason, HealthReader},
    lifecycle::{LifecycleStatus, Readiness as LifecycleReadiness},
    validation,
};
use std::sync::Arc;

/// A complete point-in-time process-readiness decision.
///
/// Intentionally exhaustive: adding a top-level decision requires a compatible
/// API release and review of every adapter's response policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadinessDecision {
    /// Lifecycle and dependency observations both establish readiness, and
    /// every application condition is satisfied.
    Ready,
    /// The process is not ready for the contained reason.
    Unready(ReadinessUnreadyReason),
}

impl ReadinessDecision {
    /// Whether this decision establishes readiness.
    pub const fn is_ready(self) -> bool {
        matches!(self, Self::Ready)
    }

    /// The reason for an unready decision, if any.
    pub const fn unready_reason(self) -> Option<ReadinessUnreadyReason> {
        match self {
            Self::Ready => None,
            Self::Unready(reason) => Some(reason),
        }
    }
}

/// Sanitized reason for a point-in-time unready decision, with no cause data.
///
/// Intentionally exhaustive: new reason variants require a compatible API
/// release and review of consumers' readiness policy. A dependency reason can
/// contain only [`DependencyUnreadyReason`]; a healthy observation has no such
/// representation. A condition reason contains only the validated name of an
/// application condition, and is decided only while lifecycle and dependency
/// health are both ready.
///
/// ```compile_fail,E0308
/// use batter_core::{
///     health::HealthStatus,
///     readiness::ReadinessUnreadyReason,
/// };
///
/// let reason = ReadinessUnreadyReason::Dependency(HealthStatus::Healthy);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadinessUnreadyReason {
    /// Application approval, driver start, or component acknowledgement is outstanding.
    Starting,
    /// Admission has closed and shutdown is underway.
    Draining,
    /// The process coordinator has stopped.
    Stopped,
    /// Lifecycle is Ready, but dependency health does not establish readiness.
    Dependency(DependencyUnreadyReason),
    /// Lifecycle and dependency health are Ready, but this application
    /// condition is not satisfied.
    Condition(ReadinessCondition),
}

/// Validated name of an application readiness condition.
///
/// A name is 1–96 ASCII alphanumeric, `.`, `_` or `-` bytes, like a registered
/// component name, so an unsatisfied condition can be matched and rendered
/// without cause data. [`ReadinessEvaluator::with_condition`] attaches its
/// check, and an unsatisfied check reports the name in
/// [`ReadinessUnreadyReason::Condition`].
///
/// ```
/// use batter_core::readiness::{ReadinessCondition, ReadinessConditionError};
///
/// let leases = ReadinessCondition::new("key-leases")?;
/// assert_eq!(leases.as_str(), "key-leases");
/// assert_eq!(
///     ReadinessCondition::new("key leases"),
///     Err(ReadinessConditionError::InvalidName),
/// );
/// # Ok::<(), ReadinessConditionError>(())
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ReadinessCondition(&'static str);

impl ReadinessCondition {
    /// Validate a condition name. A rejected name is not retained.
    pub fn new(name: &'static str) -> Result<Self, ReadinessConditionError> {
        validation::name(name).map_err(|_| ReadinessConditionError::InvalidName)?;
        Ok(Self(name))
    }

    /// Return the validated name.
    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

/// Sanitized failure to name a [`ReadinessCondition`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ReadinessConditionError {
    /// Names must be 1–96 ASCII alphanumeric, dot, underscore, or hyphen bytes.
    #[error("invalid readiness condition name")]
    InvalidName,
}

/// Read-only lifecycle, dependency and application-condition readiness evaluation.
///
/// Reads never invoke the dependency probe, refresh a timestamp, retain writer
/// ownership, or mutate lifecycle state. Dependency health is sampled first,
/// application conditions next and lifecycle last, so an observed drain
/// overrides cached success. This is a point-in-time decision, not atomic with
/// a later transition.
///
/// ```
/// use batter_core::{
///     health::{HealthMonitor, HealthPolicy},
///     lifecycle::ShutdownHandle,
///     readiness::{ReadinessDecision, ReadinessEvaluator, ReadinessUnreadyReason},
/// };
/// use std::{convert::Infallible, time::Duration};
///
/// let second = Duration::from_secs(1);
/// let policy = HealthPolicy::new(second, second, second * 3, second).unwrap();
/// let monitor = HealthMonitor::new(policy, || async { Ok::<_, Infallible>(()) });
/// let control = ShutdownHandle::new_unapproved();
/// let evaluator = ReadinessEvaluator::new(control.status(), monitor.reader());
/// assert_eq!(
///     evaluator.decision(),
///     ReadinessDecision::Unready(ReadinessUnreadyReason::Starting),
/// );
/// ```
pub struct ReadinessEvaluator<E> {
    lifecycle: LifecycleStatus,
    dependency: HealthReader<E>,
    conditions: Arc<[Condition]>,
}

/// One application check and the name that reports it.
#[derive(Clone)]
struct Condition {
    name: ReadinessCondition,
    satisfied: Arc<dyn Fn() -> bool + Send + Sync>,
}

impl<E> Clone for ReadinessEvaluator<E> {
    fn clone(&self) -> Self {
        Self {
            lifecycle: self.lifecycle.clone(),
            dependency: self.dependency.clone(),
            conditions: self.conditions.clone(),
        }
    }
}

impl<E> ReadinessEvaluator<E> {
    /// Combine purpose-qualified lifecycle and dependency observers.
    pub fn new(lifecycle: LifecycleStatus, dependency: HealthReader<E>) -> Self {
        Self {
            lifecycle,
            dependency,
            conditions: Arc::new([]),
        }
    }

    /// Narrow readiness with an application condition.
    ///
    /// `satisfied` returns whether the condition currently holds. It runs
    /// synchronously within [`Self::decision`], only after the dependency sample
    /// establishes readiness and before the final lifecycle read, so it must
    /// read already-available state without blocking or performing I/O. A
    /// condition can only narrow readiness: while it is unsatisfied, a decision
    /// that would otherwise be [`ReadinessDecision::Ready`] is
    /// `Unready(ReadinessUnreadyReason::Condition(condition))`. It is not asked
    /// while the dependency is unready, cannot replace a lifecycle or dependency
    /// reason, and returns nothing that could establish readiness. Conditions
    /// accumulate and are checked in the order added; the first unsatisfied one
    /// is reported and later checks do not run. Checks added under one name
    /// report the same condition.
    ///
    /// ```
    /// use batter_core::{
    ///     health::{HealthMonitor, HealthPolicy},
    ///     lifecycle::ShutdownHandle,
    ///     readiness::{
    ///         ReadinessCondition, ReadinessDecision, ReadinessEvaluator, ReadinessUnreadyReason,
    ///     },
    /// };
    /// use std::{
    ///     convert::Infallible,
    ///     sync::{Arc, atomic::{AtomicBool, Ordering}},
    ///     time::Duration,
    /// };
    ///
    /// let second = Duration::from_secs(1);
    /// let policy = HealthPolicy::new(second, second, second * 3, second).unwrap();
    /// let monitor = HealthMonitor::new(policy, || async { Ok::<_, Infallible>(()) });
    /// let control = ShutdownHandle::new_unapproved();
    /// let leases_valid = Arc::new(AtomicBool::new(false));
    /// let checked = leases_valid.clone();
    /// let evaluator = ReadinessEvaluator::new(control.status(), monitor.reader())
    ///     .with_condition(ReadinessCondition::new("key-leases")?, move || {
    ///         checked.load(Ordering::Acquire)
    ///     });
    /// // A condition cannot replace a lifecycle reason, whether or not it holds.
    /// for valid in [false, true] {
    ///     leases_valid.store(valid, Ordering::Release);
    ///     assert_eq!(
    ///         evaluator.decision(),
    ///         ReadinessDecision::Unready(ReadinessUnreadyReason::Starting),
    ///     );
    /// }
    /// # Ok::<(), batter_core::readiness::ReadinessConditionError>(())
    /// ```
    ///
    /// A check reports only whether its condition holds; it cannot return a
    /// decision:
    ///
    /// ```compile_fail,E0308
    /// use batter_core::readiness::{ReadinessCondition, ReadinessDecision, ReadinessEvaluator};
    ///
    /// fn forced(evaluator: ReadinessEvaluator<std::io::Error>, name: ReadinessCondition) {
    ///     let _ = evaluator.with_condition(name, || ReadinessDecision::Ready);
    /// }
    /// ```
    pub fn with_condition<F>(self, condition: ReadinessCondition, satisfied: F) -> Self
    where
        F: Fn() -> bool + Send + Sync + 'static,
    {
        let Self {
            lifecycle,
            dependency,
            conditions,
        } = self;
        let conditions = conditions
            .iter()
            .cloned()
            .chain([Condition {
                name: condition,
                satisfied: Arc::new(satisfied),
            }])
            .collect();
        Self {
            lifecycle,
            dependency,
            conditions,
        }
    }

    /// Obtain a fresh read-only decision without invoking the dependency probe.
    pub fn decision(&self) -> ReadinessDecision {
        sample_and_classify(
            || observe(self.dependency.snapshot().readiness(), &self.conditions),
            || self.lifecycle.readiness(),
        )
    }
}

/// What dependency health and application conditions establish before the
/// final lifecycle read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Observed {
    Ready,
    Dependency(DependencyUnreadyReason),
    Condition(ReadinessCondition),
}

/// Ask application conditions, in order, only when the dependency is ready.
fn observe(dependency: DependencyReadiness, conditions: &[Condition]) -> Observed {
    match dependency {
        DependencyReadiness::Unready(reason) => Observed::Dependency(reason),
        DependencyReadiness::Ready => conditions
            .iter()
            .find(|condition| !(condition.satisfied)())
            .map_or(Observed::Ready, |condition| {
                Observed::Condition(condition.name)
            }),
    }
}

fn sample_and_classify<O, L>(observe: O, lifecycle: L) -> ReadinessDecision
where
    O: FnOnce() -> Observed,
    L: FnOnce() -> LifecycleReadiness,
{
    let observed = observe();
    classify(lifecycle(), observed)
}

const fn classify(lifecycle: LifecycleReadiness, observed: Observed) -> ReadinessDecision {
    match lifecycle {
        LifecycleReadiness::Starting => {
            ReadinessDecision::Unready(ReadinessUnreadyReason::Starting)
        }
        LifecycleReadiness::Ready => match observed {
            Observed::Ready => ReadinessDecision::Ready,
            Observed::Dependency(reason) => {
                ReadinessDecision::Unready(ReadinessUnreadyReason::Dependency(reason))
            }
            Observed::Condition(condition) => {
                ReadinessDecision::Unready(ReadinessUnreadyReason::Condition(condition))
            }
        },
        LifecycleReadiness::Draining => {
            ReadinessDecision::Unready(ReadinessUnreadyReason::Draining)
        }
        LifecycleReadiness::Stopped => ReadinessDecision::Unready(ReadinessUnreadyReason::Stopped),
    }
}

#[cfg(test)]
mod tests;
