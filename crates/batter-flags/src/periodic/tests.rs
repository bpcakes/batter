//! Paused-clock contracts through the actual supervisor and crate reader.
#![allow(clippy::unwrap_used, reason = "a panic fails the test")]
#![allow(clippy::missing_assert_message, reason = "test names locate failures")]

mod destruction;
mod support;

use std::{
    future::{pending, ready},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use batter_core::{
    ConfigurationError, RegistrationError,
    lifecycle::{Readiness, ShutdownFailure, TaskOutcome},
    periodic::{PeriodicCompletion, PeriodicFailure, PeriodicRun},
};
use batter_flags_core::{PrePublication, SnapshotView, Source, SwitchState, Toggle, Version};
use tokio::{
    sync::{Semaphore, mpsc},
    time::{Instant, sleep, timeout},
};

use super::{GateRegistrationError, GatedPeriodicPolicy};
use support::*;

fn policy() -> GatedPeriodicPolicy {
    GatedPeriodicPolicy::new(SECOND, SECOND).unwrap()
}

#[test]
fn invalid_timing_is_refused_before_a_policy_exists() {
    assert_eq!(
        GatedPeriodicPolicy::new(Duration::ZERO, SECOND),
        Err(ConfigurationError::Zero("periodic interval"))
    );
    assert_eq!(
        GatedPeriodicPolicy::new(SECOND, Duration::ZERO),
        Err(ConfigurationError::Zero("periodic run budget"))
    );
    assert_eq!(
        GatedPeriodicPolicy::new(Duration::MAX, SECOND),
        Err(ConfigurationError::TooLarge("periodic interval"))
    );
    assert_eq!(
        GatedPeriodicPolicy::new(SECOND, Duration::MAX),
        Err(ConfigurationError::TooLarge("periodic run budget"))
    );
    let long = GatedPeriodicPolicy::new(SECOND, SECOND * 2).unwrap();
    assert_eq!(long.interval(), SECOND);
    assert_eq!(long.run_budget(), SECOND * 2);
}

#[tokio::test(start_paused = true)]
async fn both_unpublished_policies_and_tripped_runs_skip_without_calling_the_factory() {
    for pre_publication in [PrePublication::Unready, PrePublication::ServeDefaults] {
        let mut f = Fixture::new(pre_publication);
        let mut supervisor = supervisor();
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = calls.clone();
        let halt = f.halt.clone();
        let reader = f
            .flags
            .gate_periodic_in(
                &mut supervisor,
                "storage.pruning",
                policy(),
                &f.halt,
                move |_, view| {
                    // Count the synchronous factory, not merely polling its future.
                    counted.fetch_add(1, Ordering::SeqCst);
                    assert_eq!(view.switch(&halt).value(), SwitchState::Released);
                    ready(success())
                },
            )
            .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_eq!(reader.skipped(), 0);
        let observer = reader.clone();
        let running = supervisor.start();
        running.status().wait_ready().await.unwrap();
        sleep(Duration::from_millis(2500)).await;
        assert_eq!(reader.skipped(), 3);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        // A skip is core's own evidence, not a success.
        assert_eq!(reader.snapshot().succeeded, 0);
        assert_eq!(reader.snapshot().skipped, 3);
        assert_eq!(running.status().readiness(), Readiness::Ready);

        f.publish(1, SwitchState::Engaged, Toggle::Off);
        sleep(SECOND).await;
        assert_eq!(observer.skipped(), 4);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        f.publish(2, SwitchState::Released, Toggle::On);
        sleep(SECOND).await;
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(reader.skipped(), 4);
        drop(reader);
        let success = running.shutdown_checked().await.unwrap();
        let summary = &success.report().periodic[0].summary;
        assert_eq!(summary.invocations, 5);
        assert_eq!((summary.succeeded, summary.skipped), (1, 4));
        assert_eq!(summary.completion, PeriodicCompletion::Stopped);
        assert_eq!(observer.skipped(), 4);
        assert_eq!(observer.name(), "storage.pruning");
    }
}

/// Work reports its own view both before and after it waits.
enum Observation {
    Started { source: Source, value: Toggle },
    Finished(SnapshotView),
}

#[tokio::test(start_paused = true)]
async fn a_publication_during_work_reaches_only_the_next_run_and_never_interrupts_work() {
    let mut f = Fixture::new(PrePublication::ServeDefaults);
    f.publish(1, SwitchState::Released, Toggle::Off);
    let mut supervisor = supervisor();
    let permits = Arc::new(Semaphore::new(0));
    let release = permits.clone();
    let (events, mut observations) = mpsc::unbounded_channel();
    let flag = f.flag.clone();
    let reader = f
        .flags
        .gate_periodic_in(
            &mut supervisor,
            "storage.pruning",
            GatedPeriodicPolicy::new(SECOND * 10, SECOND * 30).unwrap(),
            &f.halt,
            move |context, view| {
                assert_eq!(context.deadline(), Instant::now() + SECOND * 30);
                events
                    .send(Observation::Started {
                        source: view.source(),
                        value: view.evaluate(&flag).value(),
                    })
                    .unwrap();
                let events = events.clone();
                let permits = permits.clone();
                async move {
                    permits.acquire().await.unwrap().forget();
                    events.send(Observation::Finished(view)).unwrap();
                    success()
                }
            },
        )
        .unwrap();
    let running = supervisor.start();
    running.status().wait_ready().await.unwrap();
    let Observation::Started {
        source: first,
        value,
    } = observations.recv().await.unwrap()
    else {
        panic!("first run must start")
    };
    assert_eq!(value, Toggle::Off);
    f.publish(2, SwitchState::Released, Toggle::On);
    release.add_permits(1);
    let Observation::Finished(finished) = observations.recv().await.unwrap() else {
        panic!("first run must finish")
    };
    assert_eq!(finished.source(), first);
    assert_eq!(finished.evaluate(&f.flag).value(), Toggle::Off);
    // No manual capture: the next scheduled factory receives the new view.
    let Observation::Started {
        source: second,
        value,
    } = timeout(SECOND * 11, observations.recv())
        .await
        .unwrap()
        .unwrap()
    else {
        panic!("second run must start")
    };
    assert!(matches!(second, Source::ControlPlane { version, .. } if version == Version::new(2)));
    assert_eq!(value, Toggle::On);

    f.publish(3, SwitchState::Engaged, Toggle::Off);
    // Keep work active across its next tick: a trip neither interrupts it nor
    // permits an overlapping invocation.
    sleep(SECOND * 11).await;
    assert_eq!(reader.snapshot().invocations, 2);
    assert_eq!(reader.skipped(), 0);
    assert!(observations.try_recv().is_err());
    release.add_permits(1);
    let Observation::Finished(finished) = observations.recv().await.unwrap() else {
        panic!("second run must finish")
    };
    assert_eq!(finished.source(), second);
    assert_eq!(finished.switch(&f.halt).value(), SwitchState::Released);
    sleep(SECOND * 10).await;
    assert!(reader.skipped() >= 1);
    assert!(observations.try_recv().is_err());
    drop(running.shutdown_checked().await.unwrap());
}

#[tokio::test(start_paused = true)]
async fn foreign_and_unowned_switches_are_refused_before_native_registration() {
    let f = Fixture::new(PrePublication::ServeDefaults);
    let foreign = Fixture::new(PrePublication::ServeDefaults).halt;
    let mut supervisor = supervisor();
    for (switch, expected) in [
        (
            &foreign,
            GateRegistrationError::ForeignHandle {
                key: foreign.key().clone(),
            },
        ),
        (
            &f.unowned,
            GateRegistrationError::NotOwned {
                key: f.unowned.key().clone(),
            },
        ),
    ] {
        for name in ["", "storage.pruning"] {
            // Invalid name would fail core registration first if validation
            // were ordered incorrectly; a valid name must remain reusable.
            let error = f
                .flags
                .gate_periodic_in(
                    &mut supervisor,
                    name,
                    policy(),
                    switch,
                    |_, _| -> std::future::Ready<PeriodicRun<std::io::Error>> {
                        panic!("rejected factory ran")
                    },
                )
                .unwrap_err();
            assert_eq!(error, expected);
        }
    }
    f.flags
        .gate_periodic_in(
            &mut supervisor,
            "storage.pruning",
            policy(),
            &f.halt,
            |_, _| ready(success()),
        )
        .unwrap();
    let running = supervisor.start();
    let success = running.shutdown_checked().await.unwrap();
    assert_eq!(success.report().periodic.len(), 1);
    assert_eq!(success.report().tasks.len(), 1);
}

#[tokio::test(start_paused = true)]
async fn native_registration_errors_survive_and_unstarted_work_is_inert() {
    let mut f = Fixture::new(PrePublication::Unready);
    f.publish(1, SwitchState::Released, Toggle::Off);
    let mut supervisor = supervisor();
    let forbidden = |_, _| -> std::future::Ready<PeriodicRun<std::io::Error>> {
        panic!("unstarted factory ran")
    };
    assert_eq!(
        f.flags
            .gate_periodic_in(&mut supervisor, "", policy(), &f.halt, forbidden)
            .unwrap_err(),
        GateRegistrationError::Registration(RegistrationError::InvalidName)
    );
    let reader = f
        .flags
        .gate_periodic_in(
            &mut supervisor,
            "storage.pruning",
            policy(),
            &f.halt,
            forbidden,
        )
        .unwrap();
    assert_eq!(
        f.flags
            .gate_periodic_in(
                &mut supervisor,
                "storage.pruning",
                policy(),
                &f.halt,
                forbidden
            )
            .unwrap_err(),
        GateRegistrationError::Registration(RegistrationError::Duplicate("storage.pruning"))
    );
    let driver = supervisor.run_until(pending::<()>());
    drop(driver);
    assert_eq!(reader.snapshot().invocations, 0);
    assert_eq!(reader.snapshot().completion, PeriodicCompletion::Pending);
    assert_eq!(reader.skipped(), 0);
}

#[derive(Debug, thiserror::Error)]
#[error("private maintenance failure {0}")]
struct Failed(u8);

#[tokio::test(start_paused = true)]
async fn recoverable_and_fatal_errors_keep_native_history_across_skips() {
    let mut f = Fixture::new(PrePublication::Unready);
    let mut supervisor = supervisor();
    // This is the work protocol, not boolean/Option combinations: one
    // recoverable attempt, then a fatal attempt on the next admitted run.
    enum Attempt {
        Recover,
        Fail,
    }
    let mut attempt = Attempt::Recover;
    let reader = f
        .flags
        .gate_periodic_in(
            &mut supervisor,
            "storage.pruning",
            policy(),
            &f.halt,
            move |_, _| {
                ready(match attempt {
                    Attempt::Recover => {
                        attempt = Attempt::Fail;
                        Err(PeriodicFailure::Recoverable(Failed(1)))
                    }
                    Attempt::Fail => Err(PeriodicFailure::Fatal(Failed(2))),
                })
            },
        )
        .unwrap();
    let running = supervisor.start();
    running.status().wait_ready().await.unwrap();
    sleep(Duration::from_millis(500)).await;
    assert_eq!(reader.skipped(), 1);
    f.publish(1, SwitchState::Released, Toggle::Off);
    sleep(SECOND).await;
    assert_eq!(reader.snapshot().recoverable_failures, 1);
    assert_eq!(running.status().readiness(), Readiness::Ready);
    f.publish(2, SwitchState::Engaged, Toggle::Off);
    sleep(SECOND).await;
    assert_eq!(reader.skipped(), 2);
    f.publish(3, SwitchState::Released, Toggle::Off);
    let ShutdownFailure::Report(report) = running.wait_checked().await.unwrap_err() else {
        panic!("expected fatal component report")
    };
    assert_eq!(report.tasks[0].outcome, TaskOutcome::Failed);
    let summary = &report.periodic[0].summary;
    assert_eq!(summary.invocations, 4);
    assert_eq!((summary.succeeded, summary.skipped), (0, 2));
    assert_eq!(summary.recoverable_failures, 1);
    assert_eq!(summary.completion, PeriodicCompletion::Fatal);
    assert_eq!(
        summary
            .first_failure
            .as_ref()
            .unwrap()
            .error
            .downcast_ref::<Failed>()
            .unwrap()
            .0,
        1
    );
    assert_eq!(
        summary
            .terminal_failure
            .as_ref()
            .unwrap()
            .error
            .downcast_ref::<Failed>()
            .unwrap()
            .0,
        2
    );
    assert!(!format!("{report:?}").contains("private maintenance failure"));
    assert_eq!(reader.skipped(), 2);
}

/// Records that the active work future was actually destroyed.
struct Destroyed(Arc<AtomicUsize>);
impl Drop for Destroyed {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[tokio::test(start_paused = true)]
async fn deadline_and_drain_destroy_active_work_and_cleanup_observes_destruction() {
    let mut f = Fixture::new(PrePublication::Unready);
    f.publish(1, SwitchState::Released, Toggle::Off);
    let mut supervisor = supervisor();
    let destroyed = Arc::new(AtomicUsize::new(0));
    let drops = destroyed.clone();
    let reader = f
        .flags
        .gate_periodic_in(
            &mut supervisor,
            "storage.pruning",
            GatedPeriodicPolicy::new(SECOND * 2, SECOND).unwrap(),
            &f.halt,
            move |_, _| {
                let guard = Destroyed(drops.clone());
                async move {
                    pending::<()>().await;
                    drop(guard);
                    success()
                }
            },
        )
        .unwrap();
    let cleanup_drops = destroyed.clone();
    supervisor
        .on_cleanup("storage", move || async move {
            assert_eq!(cleanup_drops.load(Ordering::SeqCst), 2);
            Ok(())
        })
        .unwrap();
    let running = supervisor.start();
    running.status().wait_ready().await.unwrap();
    sleep(Duration::from_millis(2500)).await;
    assert_eq!(reader.snapshot().deadline_exceeded, 1);
    assert_eq!(destroyed.load(Ordering::SeqCst), 1);
    assert_eq!(reader.snapshot().invocations, 2);
    let success = running.shutdown_checked().await.unwrap();
    assert_eq!(destroyed.load(Ordering::SeqCst), 2);
    assert_eq!(reader.snapshot().stop_interrupted, 1);
    assert_eq!(reader.snapshot().completion, PeriodicCompletion::Stopped);
    assert_eq!(reader.skipped(), 0);
    assert_eq!(success.report().cleanup.records.len(), 1);
}
