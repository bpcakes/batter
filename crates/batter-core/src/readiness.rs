//! Foundation-owned process readiness over lifecycle and dependency observations.
//!
//! The evaluator performs no probe I/O. It samples dependency health first and
//! lifecycle second, so a lifecycle transition observed during the decision
//! overrides an earlier dependency result. HTTP status and telemetry severity
//! remain adapter policy.

use crate::{
    health::{DependencyReadiness, DependencyUnreadyReason, HealthReader},
    lifecycle::{LifecycleStatus, Readiness as LifecycleReadiness},
};

/// A complete point-in-time process-readiness decision.
///
/// Intentionally exhaustive: adding a top-level decision requires a compatible
/// API release and review of every adapter's response policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadinessDecision {
    /// Lifecycle and dependency observations both establish readiness.
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
/// representation.
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
}

/// Read-only lifecycle and dependency readiness evaluation.
///
/// Reads never invoke the probe, refresh a timestamp, retain writer ownership,
/// or mutate lifecycle state. Dependency health is sampled before lifecycle so
/// an observed drain overrides cached success. This is a point-in-time decision,
/// not atomic with a later transition.
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
}

impl<E> Clone for ReadinessEvaluator<E> {
    fn clone(&self) -> Self {
        Self {
            lifecycle: self.lifecycle.clone(),
            dependency: self.dependency.clone(),
        }
    }
}

impl<E> ReadinessEvaluator<E> {
    /// Combine purpose-qualified lifecycle and dependency observers.
    pub fn new(lifecycle: LifecycleStatus, dependency: HealthReader<E>) -> Self {
        Self {
            lifecycle,
            dependency,
        }
    }

    /// Obtain a fresh read-only decision without invoking the dependency probe.
    pub fn decision(&self) -> ReadinessDecision {
        sample_and_classify(
            || self.dependency.snapshot().readiness(),
            || self.lifecycle.readiness(),
        )
    }
}

fn sample_and_classify<D, L>(dependency: D, lifecycle: L) -> ReadinessDecision
where
    D: FnOnce() -> DependencyReadiness,
    L: FnOnce() -> LifecycleReadiness,
{
    let dependency = dependency();
    classify(lifecycle(), dependency)
}

const fn classify(
    lifecycle: LifecycleReadiness,
    dependency: DependencyReadiness,
) -> ReadinessDecision {
    match lifecycle {
        LifecycleReadiness::Starting => {
            ReadinessDecision::Unready(ReadinessUnreadyReason::Starting)
        }
        LifecycleReadiness::Ready => match dependency {
            DependencyReadiness::Ready => ReadinessDecision::Ready,
            DependencyReadiness::Unready(reason) => {
                ReadinessDecision::Unready(ReadinessUnreadyReason::Dependency(reason))
            }
        },
        LifecycleReadiness::Draining => {
            ReadinessDecision::Unready(ReadinessUnreadyReason::Draining)
        }
        LifecycleReadiness::Stopped => ReadinessDecision::Unready(ReadinessUnreadyReason::Stopped),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::health::HealthStatus;
    use std::cell::Cell;

    #[test]
    fn dependency_is_sampled_before_lifecycle_and_lifecycle_wins() {
        let next = Cell::new(0);
        let decision = sample_and_classify(
            || {
                assert_eq!(next.get(), 0);
                next.set(1);
                DependencyReadiness::Ready
            },
            || {
                assert_eq!(next.get(), 1);
                next.set(2);
                LifecycleReadiness::Draining
            },
        );

        assert_eq!(next.get(), 2);
        assert_eq!(
            decision,
            ReadinessDecision::Unready(ReadinessUnreadyReason::Draining)
        );
    }

    #[test]
    fn every_lifecycle_and_health_state_has_an_explicit_decision() {
        let dependencies = [
            (
                HealthStatus::Unknown,
                DependencyReadiness::Unready(DependencyUnreadyReason::Unknown),
            ),
            (HealthStatus::Healthy, DependencyReadiness::Ready),
            (
                HealthStatus::Failed,
                DependencyReadiness::Unready(DependencyUnreadyReason::ProbeFailed),
            ),
            (
                HealthStatus::TimedOut,
                DependencyReadiness::Unready(DependencyUnreadyReason::ProbeTimedOut),
            ),
            (
                HealthStatus::Stale,
                DependencyReadiness::Unready(DependencyUnreadyReason::Stale),
            ),
            (
                HealthStatus::Stopped,
                DependencyReadiness::Unready(DependencyUnreadyReason::WriterStopped),
            ),
        ];

        for (status, dependency) in dependencies {
            assert_eq!(status.readiness(), dependency);
            assert_eq!(
                classify(LifecycleReadiness::Starting, dependency),
                ReadinessDecision::Unready(ReadinessUnreadyReason::Starting),
            );
            assert_eq!(
                classify(LifecycleReadiness::Draining, dependency),
                ReadinessDecision::Unready(ReadinessUnreadyReason::Draining),
            );
            assert_eq!(
                classify(LifecycleReadiness::Stopped, dependency),
                ReadinessDecision::Unready(ReadinessUnreadyReason::Stopped),
            );

            let expected = match dependency {
                DependencyReadiness::Ready => ReadinessDecision::Ready,
                DependencyReadiness::Unready(reason) => {
                    ReadinessDecision::Unready(ReadinessUnreadyReason::Dependency(reason))
                }
            };
            assert_eq!(classify(LifecycleReadiness::Ready, dependency), expected);
        }
    }

    #[test]
    fn decision_observation_preserves_ready_and_unready_structure() {
        assert!(ReadinessDecision::Ready.is_ready());
        assert_eq!(ReadinessDecision::Ready.unready_reason(), None);

        let decision = ReadinessDecision::Unready(ReadinessUnreadyReason::Dependency(
            DependencyUnreadyReason::ProbeFailed,
        ));
        assert!(!decision.is_ready());
        assert_eq!(
            decision.unready_reason(),
            Some(ReadinessUnreadyReason::Dependency(
                DependencyUnreadyReason::ProbeFailed
            )),
        );
    }
}
