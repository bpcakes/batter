//! Paused-time contracts of the control-plane loader over a scripted store.
#![allow(clippy::unwrap_used, reason = "a panic fails the test")]
#![allow(clippy::missing_assert_message, reason = "test names locate failures")]

mod child_root;
mod invariants;
mod last_good;
mod matrix;
mod ownership;
mod status;
mod support;
mod surfaces;

use std::time::Duration;

use batter_core::{RegistrationError, lifecycle::Readiness};
use batter_flags_core::{ErrorCause, Freshness, Reason, Source, SwitchState, Toggle, Version};
use tokio::time::sleep;

use super::{LoadCause, LoadPolicy, RegisterError, SnapshotStatus};
use crate::low_level::capture_now;
use support::*;

#[tokio::test]
async fn registration_refuses_timing_that_cannot_stay_fresh_or_fit_a_retry() {
    // A 4 s refresh budget needs a maximum age of at least 5 s + 4 s.
    let short = crate::control_plane::RefreshTiming::new(POLL, Duration::from_secs(8)).unwrap();
    let mut supervisor = supervisor();
    let (input, _) = resolved(&[]);
    let error = register(
        &mut supervisor,
        input,
        LoadPolicy::start_on_defaults(short),
        failing(LoadCause::Unavailable),
    )
    .unwrap_err();
    assert!(
        matches!(error, RegisterError::MaximumAgeTooShort { minimum } if minimum == Duration::from_secs(9))
    );

    // With a budget below the poll, the allowance must fit one poll and one run.
    let (input, _) = resolved(&[]);
    let error = register(
        &mut supervisor,
        input,
        LoadPolicy::unready_until_loaded(timing(), Duration::from_secs(8)),
        failing(LoadCause::Unavailable),
    )
    .unwrap_err();
    assert!(
        matches!(error, RegisterError::AllowanceTooShort { minimum } if minimum == Duration::from_secs(9))
    );

    // With a budget at or above the poll, it must fit one poll and two runs.
    let long_poll =
        crate::control_plane::RefreshTiming::new(Duration::from_secs(3), Duration::from_secs(60))
            .unwrap();
    let (input, _) = resolved(&[]);
    let error = register(
        &mut supervisor,
        input,
        LoadPolicy::unready_until_loaded(long_poll, Duration::from_secs(10)),
        failing(LoadCause::Unavailable),
    )
    .unwrap_err();
    assert!(
        matches!(error, RegisterError::AllowanceTooShort { minimum } if minimum == Duration::from_secs(11))
    );

    // Nothing was registered by any refused call, so a valid one succeeds.
    let (input, _) = resolved(&[]);
    register(
        &mut supervisor,
        input,
        LoadPolicy::start_on_defaults(timing()),
        failing(LoadCause::Unavailable),
    )
    .unwrap();
}

#[tokio::test]
async fn a_second_registration_on_one_supervisor_is_a_typed_duplicate() {
    let mut supervisor = supervisor();
    let (first, _) = resolved(&[]);
    register(
        &mut supervisor,
        first,
        LoadPolicy::start_on_defaults(timing()),
        failing(LoadCause::Unavailable),
    )
    .unwrap();
    let (second, _) = resolved(&[]);
    let error = register(
        &mut supervisor,
        second,
        LoadPolicy::start_on_defaults(timing()),
        failing(LoadCause::Unavailable),
    )
    .unwrap_err();
    assert!(
        matches!(
            error,
            RegisterError::Registration(RegistrationError::Duplicate(_))
        ),
        "{error:?}"
    );
}

#[test]
fn presence_and_refresh_inputs_are_validated() {
    use crate::control_plane::{
        ApplicationName, BuildId, HeartbeatCadence, PresenceError, RefreshTiming,
    };
    let s = Duration::from_secs;
    assert!(RefreshTiming::new(Duration::ZERO, s(10)).is_err());
    assert!(RefreshTiming::new(s(10), s(5)).is_err());
    assert!(RefreshTiming::new(s(10), s(3_601)).is_err());
    assert_eq!(
        HeartbeatCadence::new(s(4), s(600)),
        Err(PresenceError::Interval)
    );
    assert_eq!(
        HeartbeatCadence::new(s(301), s(6_000)),
        Err(PresenceError::Interval)
    );
    assert_eq!(
        HeartbeatCadence::new(s(10), s(99)),
        Err(PresenceError::Retention)
    );
    assert_eq!(
        HeartbeatCadence::new(s(10), s(86_401)),
        Err(PresenceError::Retention)
    );
    assert!(HeartbeatCadence::new(s(300), s(86_400)).is_ok());
    assert_eq!(
        ApplicationName::new("app+1"),
        Err(PresenceError::InvalidName)
    );
    assert!(BuildId::new("1.2.3+abc").is_ok());
    assert_eq!(BuildId::new("has space"), Err(PresenceError::InvalidName));
}

#[tokio::test(start_paused = true)]
async fn start_on_defaults_acknowledges_after_a_failed_or_hung_first_attempt() {
    for script in [
        failing(LoadCause::Unavailable),
        super::source::fake::Script::Hang,
    ] {
        let mut supervisor = supervisor();
        let (input, handles) = resolved(&[]);
        let (flags, fake) = register(
            &mut supervisor,
            input,
            LoadPolicy::start_on_defaults(timing()),
            script,
        )
        .unwrap();
        let started = tokio::time::Instant::now();
        let running = supervisor.start();
        running.status().wait_ready().await.unwrap();
        // The allowance is the 4 s run budget plus a second.
        assert!(started.elapsed() <= Duration::from_secs(5));
        assert!(fake.reads() >= 1);
        assert!(matches!(flags.status(), SnapshotStatus::NotLoaded { .. }));
        let decision = capture_now(&flags).evaluate(&handles.flag);
        assert_eq!(decision.reason(), Reason::Default);
        assert!(matches!(decision.source(), Source::NotLoaded { .. }));
        let _ = running.shutdown().await;
    }
}

#[tokio::test(start_paused = true)]
async fn unready_until_loaded_withholds_readiness_until_a_read_is_published() {
    let mut supervisor = supervisor();
    let (input, handles) = resolved(&[]);
    let policy = LoadPolicy::unready_until_loaded(timing(), Duration::from_secs(60));
    let (flags, fake) = register(
        &mut supervisor,
        input,
        policy,
        failing(LoadCause::Unavailable),
    )
    .unwrap();
    let running = supervisor.start();
    sleep(Duration::from_secs(12)).await;
    assert_eq!(running.status().readiness(), Readiness::Starting);
    let decision = capture_now(&flags).evaluate(&handles.flag);
    assert_eq!(decision.reason(), Reason::Error);
    assert_eq!(decision.error(), Some(ErrorCause::ProviderNotReady));

    fake.set(serve(&handles, 3, SwitchState::Released, Toggle::On));
    running.status().wait_ready().await.unwrap();
    assert_eq!(flags.status(), SnapshotStatus::Fresh);
    let decision = capture_now(&flags).evaluate(&handles.flag);
    assert_eq!(decision.value(), Toggle::On);
    assert_eq!(decision.reason(), Reason::Static);
    let _ = running.shutdown().await;
}

#[tokio::test(start_paused = true)]
async fn a_quiet_control_plane_stays_fresh_and_an_outage_ages_into_stale_rules() {
    let mut supervisor = supervisor();
    let (input, handles) = resolved(&[]);
    let script = serve(&handles, 3, SwitchState::Released, Toggle::On);
    let (flags, fake) = register(
        &mut supervisor,
        input,
        LoadPolicy::start_on_defaults(timing()),
        script,
    )
    .unwrap();
    let running = supervisor.start();
    running.status().wait_ready().await.unwrap();
    for _ in 0..30 {
        sleep(SECOND).await;
        assert_eq!(
            flags.status(),
            SnapshotStatus::Fresh,
            "three maximum ages of confirmations"
        );
    }
    assert_eq!(
        capture_now(&flags).switch(&handles.stale_halt).value(),
        SwitchState::Released
    );

    fake.set(failing(LoadCause::Unavailable));
    sleep(MAXIMUM_AGE + POLL).await;
    assert_eq!(flags.status(), SnapshotStatus::Stale);
    let view = capture_now(&flags);
    assert_eq!(view.freshness(), Freshness::Stale);
    assert_eq!(
        view.switch(&handles.stale_halt).value(),
        SwitchState::Engaged
    );
    assert_eq!(
        view.switch(&handles.halt).value(),
        SwitchState::Released,
        "hold last known"
    );
    let _ = running.shutdown().await;
}

#[tokio::test(start_paused = true)]
async fn a_boot_time_outage_ages_the_declared_base_into_stale_rules() {
    let mut supervisor = supervisor();
    let (input, handles) = resolved(&[]);
    let (flags, _) = register(
        &mut supervisor,
        input,
        LoadPolicy::start_on_defaults(timing()),
        failing(LoadCause::Unavailable),
    )
    .unwrap();
    let running = supervisor.start();
    running.status().wait_ready().await.unwrap();
    sleep(MAXIMUM_AGE + SECOND).await;
    assert!(
        matches!(flags.status(), SnapshotStatus::NotLoaded { since_registration } if since_registration > MAXIMUM_AGE)
    );
    let view = capture_now(&flags);
    assert_eq!(view.freshness(), Freshness::Stale);
    assert_eq!(
        view.switch(&handles.stale_halt).value(),
        SwitchState::Engaged
    );
    assert_eq!(view.switch(&handles.halt).value(), SwitchState::Released);
    let _ = running.shutdown().await;
}

#[tokio::test(start_paused = true)]
async fn the_settings_floor_engages_a_switch_before_and_after_publication() {
    let mut supervisor = supervisor();
    let (input, handles) = resolved(&[("FLAG_HALT", "engaged")]);
    let (flags, fake) = register(
        &mut supervisor,
        input,
        LoadPolicy::start_on_defaults(timing()),
        failing(LoadCause::Unavailable),
    )
    .unwrap();
    let running = supervisor.start();
    running.status().wait_ready().await.unwrap();
    assert_eq!(
        capture_now(&flags).switch(&handles.halt).value(),
        SwitchState::Engaged
    );
    fake.set(serve(&handles, 2, SwitchState::Released, Toggle::Off));
    sleep(POLL + SECOND).await;
    assert_eq!(flags.status(), SnapshotStatus::Fresh);
    assert_eq!(
        capture_now(&flags).switch(&handles.halt).value(),
        SwitchState::Engaged
    );
    let _ = running.shutdown().await;
}

#[tokio::test(start_paused = true)]
async fn ending_the_loader_reports_loader_stopped() {
    let mut supervisor = supervisor();
    let (input, handles) = resolved(&[]);
    let script = serve(&handles, 1, SwitchState::Released, Toggle::Off);
    let (flags, _) = register(
        &mut supervisor,
        input,
        LoadPolicy::start_on_defaults(timing()),
        script,
    )
    .unwrap();
    let running = supervisor.start();
    running.status().wait_ready().await.unwrap();
    assert_eq!(flags.status(), SnapshotStatus::Fresh);
    let _ = running.shutdown().await;
    assert_eq!(flags.status(), SnapshotStatus::LoaderStopped);
    assert!(
        matches!(capture_now(&flags).source(), Source::ControlPlane { version, .. } if version == Version::new(1)),
        "the last record stays served"
    );
}

#[tokio::test]
async fn a_wake_spacing_longer_than_the_poll_is_refused() {
    use crate::control_plane::ListenEndpoint;
    let mut supervisor = supervisor();
    let (input, _) = resolved(&[]);
    let options = sqlx::postgres::PgConnectOptions::new_without_pgpass()
        .host("127.0.0.1")
        .username("flags");
    let endpoint = ListenEndpoint::new(options, POLL + SECOND).unwrap();
    let policy = LoadPolicy::start_on_defaults(timing()).with_listen(endpoint);
    let error = register(
        &mut supervisor,
        input,
        policy,
        failing(LoadCause::Unavailable),
    )
    .unwrap_err();
    assert!(
        matches!(error, RegisterError::WakeSpacingTooLong { maximum } if maximum == POLL),
        "{error:?}"
    );
}
