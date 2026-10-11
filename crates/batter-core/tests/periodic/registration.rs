//! Protected, validated and inert registration.

use super::support::*;
use batter_core::{
    ConfigurationError, RegistrationError,
    lifecycle::Readiness,
    periodic::{
        PeriodicCompletion, PeriodicPolicy, PeriodicShutdown, PeriodicStartup, register_periodic_in,
    },
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

#[test]
fn timing_is_validated_before_registration_is_even_attempted() {
    let startup = PeriodicStartup::immediate();
    let shutdown = PeriodicShutdown::StopAtDrain;
    assert_eq!(
        PeriodicPolicy::new(Duration::ZERO, SECOND, startup, shutdown),
        Err(ConfigurationError::Zero("periodic interval")),
    );
    assert_eq!(
        PeriodicPolicy::new(SECOND, Duration::ZERO, startup, shutdown),
        Err(ConfigurationError::Zero("periodic run budget")),
    );
    assert_eq!(
        PeriodicPolicy::new(Duration::from_secs(u64::MAX), SECOND, startup, shutdown),
        Err(ConfigurationError::TooLarge("periodic interval")),
    );
    assert_eq!(
        PeriodicPolicy::new(SECOND, Duration::from_secs(u64::MAX), startup, shutdown),
        Err(ConfigurationError::TooLarge("periodic run budget")),
    );
    assert_eq!(
        PeriodicStartup::after_first_success(Duration::ZERO),
        Err(ConfigurationError::Zero(
            "periodic initialization allowance"
        )),
    );
    // A run budget longer than the interval is a valid selection.
    let policy = PeriodicPolicy::new(SECOND, SECOND * 4, startup, shutdown)
        .expect("a budget longer than the interval is valid");
    assert!(policy.run_budget() > policy.interval());
}

#[tokio::test(start_paused = true)]
async fn an_invalid_or_duplicate_name_is_rejected_without_invoking_the_factory() {
    let invoked = Arc::new(AtomicBool::new(false));
    let mut supervisor = supervisor();
    let policy = immediate(SECOND, SECOND, PeriodicShutdown::StopAtDrain);
    for name in ["", "has space", "report/slash"] {
        let called = invoked.clone();
        assert_eq!(
            register_periodic_in(&mut supervisor, name, policy, move |_| {
                called.store(true, Ordering::SeqCst);
                async { succeeded() }
            })
            .err(),
            Some(RegistrationError::InvalidName),
            "{name} must be rejected",
        );
    }
    register_periodic_in(&mut supervisor, "storage.pruning", policy, |_| async {
        succeeded()
    })
    .expect("a validated name registers");
    // Periodic components share the one component-name vocabulary.
    let called = invoked.clone();
    assert_eq!(
        register_periodic_in(&mut supervisor, "storage.pruning", policy, move |_| {
            called.store(true, Ordering::SeqCst);
            async { succeeded() }
        })
        .err(),
        Some(RegistrationError::Duplicate("storage.pruning")),
    );
    supervisor
        .register(
            "storage.pruning",
            |startup| async move { Ok(startup.abandon()) },
        )
        .expect_err("a direct component cannot reuse a periodic name");
    assert!(!invoked.load(Ordering::SeqCst));
}

#[tokio::test(start_paused = true)]
async fn an_abandoned_unstarted_supervisor_runs_no_maintenance() {
    let runs = Arc::new(AtomicUsize::new(0));
    let mut supervisor = supervisor();
    let counted = runs.clone();
    let reader = register_periodic_in(
        &mut supervisor,
        "storage.pruning",
        immediate(SECOND, SECOND, PeriodicShutdown::StopAtDrain),
        move |_| {
            let counted = counted.clone();
            async move {
                counted.fetch_add(1, Ordering::SeqCst);
                succeeded()
            }
        },
    )
    .unwrap();
    let status = supervisor.status();
    drop(supervisor);
    assert_eq!(status.readiness(), Readiness::Draining);
    assert_eq!(runs.load(Ordering::SeqCst), 0);
    // The supervisor published no report, so the snapshot stays incomplete.
    let summary = reader.snapshot();
    assert_eq!(summary.completion, PeriodicCompletion::Pending);
    assert!(!summary.is_complete());
    assert_eq!(summary.invocations, 0);
}

#[tokio::test(start_paused = true)]
async fn a_never_polled_driver_runs_no_maintenance() {
    let runs = Arc::new(AtomicUsize::new(0));
    let mut supervisor = supervisor();
    let counted = runs.clone();
    register_periodic_in(
        &mut supervisor,
        "storage.pruning",
        immediate(SECOND, SECOND, PeriodicShutdown::SupportThroughDrain),
        move |_| {
            let counted = counted.clone();
            async move {
                counted.fetch_add(1, Ordering::SeqCst);
                succeeded()
            }
        },
    )
    .unwrap();
    let driver = supervisor.run_until(std::future::ready(()));
    drop(driver);
    assert_eq!(runs.load(Ordering::SeqCst), 0);
}

#[tokio::test(start_paused = true)]
async fn an_already_draining_registration_abandons_without_running() {
    let runs = Arc::new(AtomicUsize::new(0));
    let mut supervisor = supervisor();
    let counted = runs.clone();
    let reader = register_periodic_in(
        &mut supervisor,
        "storage.pruning",
        immediate(SECOND, SECOND, PeriodicShutdown::SupportThroughDrain),
        move |_| {
            let counted = counted.clone();
            async move {
                counted.fetch_add(1, Ordering::SeqCst);
                succeeded()
            }
        },
    )
    .unwrap();
    supervisor.handle().request();
    let report = supervisor.run_until(std::future::pending()).await;
    assert_eq!(runs.load(Ordering::SeqCst), 0);
    assert_eq!(
        reader.snapshot().completion,
        PeriodicCompletion::AbandonedDuringStartup,
    );
    // Readiness stayed gated on a component that never initialized.
    assert!(report.is_success());
    assert_eq!(report.periodic.len(), 1);
    assert!(!report.periodic[0].summary.acknowledged);
}
