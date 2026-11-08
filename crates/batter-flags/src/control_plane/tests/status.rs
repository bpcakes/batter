//! Every status arm, each gate's answer, and the load events.

use std::time::Duration;

use batter_flags_core::{
    SwitchState, Toggle, Version,
    low_level::{CandidateRow, Refusal, SnapshotRejection},
};
use tokio::time::sleep;

use super::super::{
    Gate, LoadCause, LoadPolicy, RejectionCause, SnapshotStatus, source::fake::Script,
};
use super::support::*;
use crate::low_level::capture_now;

fn gates(flags: &super::super::RegisteredFlags) -> (bool, bool) {
    let (published, published_ok) = flags.readiness_condition(Gate::Published);
    let (fresh, fresh_ok) = flags.readiness_condition(Gate::Fresh);
    assert_eq!(published.as_str(), "flags.snapshot");
    assert_eq!(fresh.as_str(), "flags.snapshot");
    (published_ok(), fresh_ok())
}

#[tokio::test(start_paused = true)]
async fn regression_drift_and_rejection_never_renew_freshness() {
    let mut supervisor = supervisor();
    let (input, handles) = resolved(&[]);
    let script = serve(&handles, 5, SwitchState::Engaged, Toggle::On);
    let (flags, fake) = register(
        &mut supervisor,
        input,
        LoadPolicy::start_on_defaults(timing()),
        script,
    )
    .unwrap();
    let running = supervisor.start();
    running.status().wait_ready().await.unwrap();
    assert_eq!(flags.status(), SnapshotStatus::Fresh);
    assert_eq!(gates(&flags), (true, true));

    // A read below the applied version is a regression.
    fake.set(serve(&handles, 4, SwitchState::Released, Toggle::Off));
    sleep(POLL + SECOND).await;
    assert_eq!(
        flags.status(),
        SnapshotStatus::Regressed {
            applied: Version::new(5),
            observed: Version::new(4)
        }
    );
    assert!(matches!(
        flags.last_rejection(),
        Some(Refusal::Regressed { .. })
    ));
    assert_eq!(
        capture_now(&flags).switch(&handles.halt).value(),
        SwitchState::Engaged
    );
    sleep(MAXIMUM_AGE).await;
    assert_eq!(
        capture_now(&flags).freshness(),
        batter_flags_core::Freshness::Stale,
        "no renewal"
    );
    assert_eq!(gates(&flags), (true, false));

    // The applied version with other content is drift.
    fake.set(serve(&handles, 5, SwitchState::Released, Toggle::On));
    sleep(POLL + SECOND).await;
    assert_eq!(
        flags.status(),
        SnapshotStatus::Rejected {
            observed: Some(Version::new(5)),
            cause: RejectionCause::DigestDrift
        }
    );
    assert_eq!(
        capture_now(&flags).switch(&handles.halt).value(),
        SwitchState::Engaged
    );

    // A structurally invalid read is rejected; the served record stays.
    let at = Version::new(6);
    fake.set(Script::Serve {
        version: at,
        rows: vec![
            CandidateRow::switch(&handles.halt, SwitchState::Released, at),
            CandidateRow::switch(&handles.halt, SwitchState::Released, at),
        ],
    });
    sleep(POLL + SECOND).await;
    assert_eq!(
        flags.status(),
        SnapshotStatus::Rejected {
            observed: Some(at),
            cause: RejectionCause::Structural(SnapshotRejection::DuplicateKey { row: 1 })
        }
    );
    // An outage does not clear a rejection; a publication does.
    fake.set(failing(LoadCause::Unavailable));
    sleep(POLL + SECOND).await;
    assert!(matches!(flags.status(), SnapshotStatus::Rejected { .. }));
    fake.set(serve(&handles, 7, SwitchState::Released, Toggle::Off));
    sleep(POLL + SECOND).await;
    assert_eq!(flags.status(), SnapshotStatus::Fresh);
    assert!(
        matches!(flags.last_rejection(), Some(Refusal::Rejected(_))),
        "the history stays"
    );
    let _ = running.shutdown().await;
}

#[tokio::test(start_paused = true)]
async fn a_rejection_before_any_publication_says_why_nothing_loads() {
    let mut supervisor = supervisor();
    let (input, handles) = resolved(&[]);
    let at = Version::new(2);
    let duplicate = Script::Serve {
        version: at,
        rows: vec![
            CandidateRow::flag(&handles.flag, Toggle::On, at),
            CandidateRow::flag(&handles.flag, Toggle::On, at),
        ],
    };
    let (flags, _) = register(
        &mut supervisor,
        input,
        LoadPolicy::start_on_defaults(timing()),
        duplicate,
    )
    .unwrap();
    let running = supervisor.start();
    running.status().wait_ready().await.unwrap();
    assert!(matches!(
        flags.status(),
        SnapshotStatus::Rejected {
            cause: RejectionCause::Structural(_),
            ..
        }
    ));
    assert_eq!(gates(&flags), (false, false), "nothing is published");
    let _ = running.shutdown().await;
}

#[tokio::test(start_paused = true)]
async fn a_store_behind_the_schema_reports_it_before_and_after_a_publication() {
    let mut supervisor = supervisor();
    let (input, handles) = resolved(&[]);
    let (flags, fake) = register(
        &mut supervisor,
        input,
        LoadPolicy::start_on_defaults(timing()),
        schema_behind(0),
    )
    .unwrap();
    let running = supervisor.start();
    running.status().wait_ready().await.unwrap();
    assert_eq!(
        flags.status(),
        SnapshotStatus::SchemaBehind {
            required: 1,
            found: 0
        }
    );
    assert_eq!(gates(&flags), (false, false));

    fake.set(serve(&handles, 1, SwitchState::Released, Toggle::On));
    sleep(POLL + SECOND).await;
    assert_eq!(
        flags.status(),
        SnapshotStatus::Fresh,
        "the first run after migration recovers"
    );
    assert_eq!(gates(&flags), (true, true));
    fake.set(schema_behind(0));
    sleep(POLL + SECOND).await;
    assert_eq!(
        flags.status(),
        SnapshotStatus::SchemaBehind {
            required: 1,
            found: 0
        }
    );
    assert_eq!(
        gates(&flags),
        (false, false),
        "a restore below the migration is unready"
    );
    let _ = running.shutdown().await;
}

#[tokio::test(start_paused = true)]
async fn not_loaded_is_unready_for_both_gates_and_a_publication_satisfies_both() {
    let mut supervisor = supervisor();
    let (input, handles) = resolved(&[]);
    let (flags, fake) = register(
        &mut supervisor,
        input,
        LoadPolicy::start_on_defaults(timing()),
        failing(LoadCause::Standby),
    )
    .unwrap();
    let running = supervisor.start();
    running.status().wait_ready().await.unwrap();
    assert!(matches!(flags.status(), SnapshotStatus::NotLoaded { .. }));
    assert_eq!(gates(&flags), (false, false));
    fake.set(serve(&handles, 1, SwitchState::Released, Toggle::On));
    sleep(POLL + SECOND).await;
    assert_eq!(gates(&flags), (true, true));
    // A standby read never renews: the view ages into Stale but stays published.
    fake.set(failing(LoadCause::Standby));
    sleep(MAXIMUM_AGE + POLL).await;
    assert_eq!(flags.status(), SnapshotStatus::Stale);
    assert_eq!(gates(&flags), (true, false));
    let _ = running.shutdown().await;
}

#[tokio::test(start_paused = true)]
async fn a_hundred_failing_polls_with_one_cause_warn_once() {
    use tracing::Level;
    let emitted = || super::super::refresh::EMITTED.with(|e| e.borrow().clone());
    let mut supervisor = supervisor();
    let (input, handles) = resolved(&[]);
    let (_flags, fake) = register(
        &mut supervisor,
        input,
        LoadPolicy::start_on_defaults(timing()),
        failing(LoadCause::Unavailable),
    )
    .unwrap();
    let running = supervisor.start();
    running.status().wait_ready().await.unwrap();
    sleep(POLL * 100).await;
    assert!(fake.reads() >= 100);
    fake.set(failing(LoadCause::Deadline));
    sleep(POLL * 3).await;
    fake.set(serve(&handles, 1, SwitchState::Released, Toggle::On));
    sleep(POLL * 2).await;
    let _ = running.shutdown().await;
    let events = emitted();
    let warnings: Vec<_> = events
        .iter()
        .filter(|(level, _)| *level == Level::WARN)
        .map(|(_, cause)| *cause)
        .collect();
    assert_eq!(
        warnings,
        ["unavailable", "deadline"],
        "one warning per cause"
    );
    assert!(events.len() >= 104);
    let infos = events
        .iter()
        .filter(|(level, _)| *level == Level::INFO)
        .count();
    assert_eq!(infos, 1, "the recovery is told once");
}

#[tokio::test(start_paused = true)]
async fn a_confirming_read_clears_a_refusal_and_a_regression_outranks_corruption() {
    let mut supervisor = supervisor();
    let (input, handles) = resolved(&[]);
    let script = serve(&handles, 5, SwitchState::Engaged, Toggle::On);
    let (flags, fake) = register(
        &mut supervisor,
        input,
        LoadPolicy::start_on_defaults(timing()),
        script,
    )
    .unwrap();
    let running = supervisor.start();
    running.status().wait_ready().await.unwrap();

    // A version-4 read that is also corrupt is a regression first.
    let at = Version::new(4);
    fake.set(Script::Serve {
        version: at,
        rows: vec![
            CandidateRow::switch(&handles.halt, SwitchState::Released, at),
            CandidateRow::switch(&handles.halt, SwitchState::Released, at),
        ],
    });
    sleep(POLL + SECOND).await;
    assert_eq!(
        flags.status(),
        SnapshotStatus::Regressed {
            applied: Version::new(5),
            observed: Version::new(4)
        }
    );
    // The applied snapshot read again confirms it and clears the refusal.
    fake.set(serve(&handles, 5, SwitchState::Engaged, Toggle::On));
    sleep(POLL).await;
    assert_eq!(flags.status(), SnapshotStatus::Fresh);
    let _ = running.shutdown().await;
}

#[tokio::test(start_paused = true)]
async fn an_unready_load_that_outlasts_its_allowance_drains() {
    let mut supervisor = supervisor();
    let (input, _) = resolved(&[]);
    let policy = LoadPolicy::unready_until_loaded(timing(), Duration::from_secs(10));
    let (_flags, _) = register(
        &mut supervisor,
        input,
        policy,
        failing(LoadCause::Unavailable),
    )
    .unwrap();
    let running = supervisor.start();
    sleep(Duration::from_secs(9)).await;
    assert_eq!(
        running.status().readiness(),
        batter_core::lifecycle::Readiness::Starting
    );
    sleep(Duration::from_secs(3)).await;
    assert!(matches!(
        running.status().readiness(),
        batter_core::lifecycle::Readiness::Draining | batter_core::lifecycle::Readiness::Stopped
    ));
    let _ = running.shutdown().await;
}

#[tokio::test(start_paused = true)]
async fn presence_skips_its_heartbeat_while_the_schema_is_behind_and_prunes_every_ten_intervals() {
    // The store is unreachable, so a heartbeat that runs fails recoverably.
    let mut supervisor = supervisor();
    let (input, handles) = resolved(&[]);
    let script = serve(&handles, 1, SwitchState::Released, Toggle::On);
    let (flags, fake) = register(
        &mut supervisor,
        input,
        LoadPolicy::start_on_defaults(timing()),
        script,
    )
    .unwrap();
    let running = supervisor.start();
    running.status().wait_ready().await.unwrap();
    sleep(Duration::from_secs(12)).await;
    let reached = flags.presence();
    assert!(
        reached.has_failures(),
        "a heartbeat reached the store and failed"
    );
    assert_eq!(
        running.status().readiness(),
        batter_core::lifecycle::Readiness::Ready
    );

    fake.set(schema_behind(0));
    sleep(POLL + SECOND).await;
    let before = flags.presence();
    sleep(Duration::from_secs(20)).await;
    let after = flags.presence();
    assert!(after.invocations > before.invocations);
    assert_eq!(
        after.recoverable_failures, before.recoverable_failures,
        "no heartbeat reached the store"
    );
    assert_eq!(
        after.deadline_exceeded, 0,
        "every store call returned within its run"
    );

    // The prune runs immediately, then every 50 s.
    assert_eq!(flags.prune().invocations, 1);
    sleep(Duration::from_secs(50)).await;
    assert_eq!(flags.prune().invocations, 2);
    assert_eq!(
        running.status().readiness(),
        batter_core::lifecycle::Readiness::Ready
    );
    let _ = running.shutdown().await;
}

#[tokio::test(start_paused = true)]
async fn a_drain_that_destroys_a_hung_run_publishes_nothing() {
    let mut supervisor = supervisor();
    let (input, _) = resolved(&[]);
    let (flags, fake) = register(
        &mut supervisor,
        input,
        LoadPolicy::start_on_defaults(timing()),
        Script::Hang,
    )
    .unwrap();
    let running = supervisor.start();
    running.status().wait_ready().await.unwrap();
    sleep(Duration::from_millis(5_500)).await;
    assert!(fake.reads() >= 2, "a second run is in flight");
    let _ = running.shutdown().await;
    assert_eq!(flags.status(), SnapshotStatus::LoaderStopped);
    assert!(!flags.shared.reader().has_publication());
}
