use super::*;
use crate::{
    health::{HealthMonitor, HealthPolicy, HealthStatus},
    lifecycle::ShutdownHandle,
};
use std::{
    convert::Infallible,
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};

const LIFECYCLES: [LifecycleReadiness; 4] = [
    LifecycleReadiness::Starting,
    LifecycleReadiness::Ready,
    LifecycleReadiness::Draining,
    LifecycleReadiness::Stopped,
];

fn dependencies() -> [(HealthStatus, DependencyReadiness); 6] {
    [
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
    ]
}

/// A condition with a fixed answer that counts how often it is asked.
fn counted(name: &'static str, satisfied: bool, asked: &Arc<AtomicUsize>) -> Condition {
    let asked = asked.clone();
    Condition {
        name: ReadinessCondition::new(name).unwrap(),
        satisfied: Arc::new(move || {
            asked.fetch_add(1, Ordering::SeqCst);
            satisfied
        }),
    }
}

#[test]
fn dependency_then_conditions_are_sampled_before_lifecycle_and_lifecycle_wins() {
    let next = Arc::new(AtomicUsize::new(0));
    let order = next.clone();
    let conditions = [Condition {
        name: ReadinessCondition::new("ordered").unwrap(),
        satisfied: Arc::new(move || {
            assert_eq!(order.swap(2, Ordering::SeqCst), 1);
            true
        }),
    }];
    let decision = sample_and_classify(
        || {
            assert_eq!(next.swap(1, Ordering::SeqCst), 0);
            observe(DependencyReadiness::Ready, &conditions)
        },
        || {
            assert_eq!(next.swap(3, Ordering::SeqCst), 2);
            LifecycleReadiness::Draining
        },
    );

    assert_eq!(next.load(Ordering::SeqCst), 3);
    assert_eq!(
        decision,
        ReadinessDecision::Unready(ReadinessUnreadyReason::Draining)
    );
}

#[test]
fn every_lifecycle_and_health_state_has_an_explicit_decision() {
    for (status, dependency) in dependencies() {
        assert_eq!(status.readiness(), dependency);
        let observed = observe(dependency, &[]);
        assert_eq!(
            classify(LifecycleReadiness::Starting, observed),
            ReadinessDecision::Unready(ReadinessUnreadyReason::Starting),
        );
        assert_eq!(
            classify(LifecycleReadiness::Draining, observed),
            ReadinessDecision::Unready(ReadinessUnreadyReason::Draining),
        );
        assert_eq!(
            classify(LifecycleReadiness::Stopped, observed),
            ReadinessDecision::Unready(ReadinessUnreadyReason::Stopped),
        );

        let expected = match dependency {
            DependencyReadiness::Ready => ReadinessDecision::Ready,
            DependencyReadiness::Unready(reason) => {
                ReadinessDecision::Unready(ReadinessUnreadyReason::Dependency(reason))
            }
        };
        assert_eq!(classify(LifecycleReadiness::Ready, observed), expected);
    }
}

#[test]
fn conditions_only_narrow_a_ready_lifecycle_and_dependency() {
    let first = ReadinessCondition::new("first").unwrap();
    let second = ReadinessCondition::new("second").unwrap();
    // Each set lists its answers in the order added, then the condition an
    // otherwise ready decision must report.
    let sets: [(&[bool], Option<ReadinessCondition>); 5] = [
        (&[], None),
        (&[true, true], None),
        (&[false, true], Some(first)),
        (&[true, false], Some(second)),
        (&[false, false], Some(first)),
    ];
    for (_, dependency) in dependencies() {
        for (answers, unsatisfied) in sets {
            let asked = Arc::new(AtomicUsize::new(0));
            let conditions: Vec<_> = answers
                .iter()
                .zip(["first", "second"])
                .map(|(&satisfied, name)| counted(name, satisfied, &asked))
                .collect();
            let observed = observe(dependency, &conditions);
            let expected_asks = match dependency {
                DependencyReadiness::Unready(_) => 0,
                DependencyReadiness::Ready => answers
                    .iter()
                    .position(|satisfied| !satisfied)
                    .map_or(answers.len(), |index| index + 1),
            };
            assert_eq!(asked.load(Ordering::SeqCst), expected_asks);

            for lifecycle in LIFECYCLES {
                let without = classify(lifecycle, observe(dependency, &[]));
                let decision = classify(lifecycle, observed);
                if without.is_ready() {
                    let expected = unsatisfied.map_or(ReadinessDecision::Ready, |condition| {
                        ReadinessDecision::Unready(ReadinessUnreadyReason::Condition(condition))
                    });
                    assert_eq!(decision, expected);
                } else {
                    // No condition answer can make an unready lifecycle or
                    // dependency ready or replace its reason.
                    assert_eq!(
                        decision, without,
                        "{lifecycle:?} {dependency:?} {answers:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn conditions_accumulate_in_order_and_clones_share_them() {
    let second = Duration::from_secs(1);
    let policy = HealthPolicy::new(second, second, second * 3, second).unwrap();
    let monitor = HealthMonitor::new(policy, || async { Ok::<_, Infallible>(()) });
    let control = ShutdownHandle::new_unapproved();
    let base = ReadinessEvaluator::new(control.status(), monitor.reader());
    let asked = Arc::new(AtomicUsize::new(0));
    let evaluator = base
        .clone()
        .with_condition(ReadinessCondition::new("first").unwrap(), || true)
        .with_condition(ReadinessCondition::new("second").unwrap(), {
            let asked = asked.clone();
            move || {
                asked.fetch_add(1, Ordering::SeqCst);
                false
            }
        });

    // Adding a condition leaves the evaluator it extends unchanged, and clones
    // share one list.
    assert!(base.conditions.is_empty());
    let names: Vec<_> = evaluator
        .conditions
        .iter()
        .map(|condition| condition.name.as_str())
        .collect();
    assert_eq!(names, ["first", "second"]);
    assert!(Arc::ptr_eq(
        &evaluator.clone().conditions,
        &evaluator.conditions
    ));
    assert_eq!(
        observe(DependencyReadiness::Ready, &evaluator.conditions),
        Observed::Condition(ReadinessCondition::new("second").unwrap()),
    );
    // Nothing has probed the dependency yet, so the decision reports the
    // lifecycle without asking any condition.
    let asked_before = asked.load(Ordering::SeqCst);
    assert_eq!(
        evaluator.decision(),
        ReadinessDecision::Unready(ReadinessUnreadyReason::Starting),
    );
    assert_eq!(asked.load(Ordering::SeqCst), asked_before);
}

#[test]
fn condition_names_are_validated_like_component_names() {
    const LONGEST: &str = concat!(
        "abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyz",
        "abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqr",
    );
    const TOO_LONG: &str = concat!(
        "abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyz",
        "abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrs",
    );
    assert_eq!(LONGEST.len(), 96);
    for valid in ["a", "key-leases", "tenant.cache_v2", LONGEST] {
        assert_eq!(ReadinessCondition::new(valid).unwrap().as_str(), valid);
    }
    for invalid in [
        "",
        "key leases",
        "key/leases",
        "clé",
        "line\nbreak",
        TOO_LONG,
    ] {
        assert_eq!(
            ReadinessCondition::new(invalid),
            Err(ReadinessConditionError::InvalidName),
            "{invalid:?}",
        );
    }
    assert_eq!(
        ReadinessConditionError::InvalidName.to_string(),
        "invalid readiness condition name",
    );
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

    let condition = ReadinessCondition::new("key-leases").unwrap();
    let decision = ReadinessDecision::Unready(ReadinessUnreadyReason::Condition(condition));
    assert!(!decision.is_ready());
    assert_eq!(
        decision.unready_reason(),
        Some(ReadinessUnreadyReason::Condition(condition)),
    );
}

#[test]
fn lifecycle_only_conditions_narrow_and_lifecycle_is_sampled_last() {
    let (control, approval) = ShutdownHandle::new_with_readiness_approval();
    let ready = ReadinessEvaluator::lifecycle_only(control.status());
    let condition = ReadinessCondition::new("application-state").unwrap();
    let closed = ready.clone().with_condition(condition, || false);
    assert_eq!(
        ready.decision(),
        ReadinessDecision::Unready(ReadinessUnreadyReason::Starting)
    );
    assert_eq!(closed.decision(), ready.decision());
    approval.approve();
    assert_eq!(ready.decision(), ReadinessDecision::Ready);
    assert_eq!(
        closed.decision(),
        ReadinessDecision::Unready(ReadinessUnreadyReason::Condition(condition))
    );
    let draining = ready.clone().with_condition(condition, {
        let control = control.clone();
        move || {
            control.request();
            true
        }
    });
    assert_eq!(
        draining.decision(),
        ReadinessDecision::Unready(ReadinessUnreadyReason::Draining)
    );
    assert_eq!(closed.decision(), draining.decision());
}
