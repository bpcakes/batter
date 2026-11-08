//! Invariants under the loader: a base an engaged governor would break is
//! refused at registration, and a hold only a stale view makes is counted
//! while the view is stale.

use batter_core::settings::SettingsSource;
use batter_flags_core::{
    CatalogBuilder, CivilDate, Expiry, KillSwitch, ModeFlag, ReleaseFlag, SafeState,
    SettingsPrefix, StaleRule, SwitchState, Toggle, Version, low_level::CandidateRow,
};
use tokio::time::sleep;

use super::super::{LoadCause, LoadPolicy, RegisterError, SnapshotStatus, source::fake::Script};
use super::support::*;
use crate::{declared, low_level::capture_now};

/// `dependent` on requires `governed` on, which `halt`, engaged when stale,
/// governs; resolved with `settings`.
fn governed(
    settings: &[(&str, &str)],
) -> (
    declared::Resolved,
    ModeFlag<Toggle>,
    ReleaseFlag<Toggle>,
    KillSwitch,
) {
    let mut catalog = CatalogBuilder::new(SettingsPrefix::new("FLAG_").unwrap());
    let serving = catalog.process_kind("serving").unwrap();
    let hosted = catalog.environment_class("hosted").unwrap();
    let halt = catalog
        .kill_switch("halt", SafeState::Released, StaleRule::EngageWhenStale)
        .declare()
        .unwrap();
    let governed = catalog
        .release_flag(
            "governed",
            Toggle::Off,
            Expiry::on(CivilDate::new(2027, 3, 31).unwrap()),
        )
        .governed_by(&halt)
        .declare()
        .unwrap();
    let dependent = catalog
        .mode_flag("dependent", Toggle::Off)
        .declare()
        .unwrap();
    catalog
        .requires(&dependent, Toggle::On, &governed, Toggle::On)
        .unwrap();
    let source = SettingsSource::from_pairs(
        settings
            .iter()
            .map(|(name, value)| ((*name).into(), (*value).into())),
    )
    .unwrap();
    let resolved = declared::resolve(
        catalog.build().unwrap(),
        &source,
        serving,
        hosted,
        CivilDate::new(2026, 11, 8).unwrap(),
    )
    .unwrap();
    (resolved, dependent, governed, halt)
}

#[tokio::test]
async fn a_base_an_engaged_governor_would_break_is_refused_at_registration() {
    let (resolved, ..) = governed(&[("FLAG_DEPENDENT", "on"), ("FLAG_GOVERNED", "on")]);
    let mut supervisor = supervisor();
    let error = register(
        &mut supervisor,
        resolved,
        LoadPolicy::start_on_defaults(timing()),
        failing(LoadCause::Unavailable),
    )
    .unwrap_err();
    assert!(
        matches!(&error, RegisterError::GovernedInvariant { flag, other, .. }
            if flag.as_str() == "dependent" && other.as_str() == "governed"),
        "{error:?}"
    );
    // Nothing was registered, so the same supervisor takes a valid base.
    let (resolved, ..) = governed(&[]);
    register(
        &mut supervisor,
        resolved,
        LoadPolicy::start_on_defaults(timing()),
        failing(LoadCause::Unavailable),
    )
    .unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_hold_only_a_stale_view_makes_is_counted_while_stale() {
    let (resolved, dependent, governed, halt) = governed(&[]);
    let at = Version::new(3);
    let script = Script::Serve {
        version: at,
        rows: vec![
            CandidateRow::flag(&dependent, Toggle::On, at),
            CandidateRow::flag(&governed, Toggle::On, at),
            CandidateRow::switch(&halt, SwitchState::Released, at),
        ],
    };
    let mut supervisor = supervisor();
    let (flags, fake) = register(
        &mut supervisor,
        resolved,
        LoadPolicy::start_on_defaults(timing()),
        script,
    )
    .unwrap();
    let running = supervisor.start();
    running.status().wait_ready().await.unwrap();
    assert_eq!(flags.status(), SnapshotStatus::Fresh);
    assert_eq!(flags.held(), 0);
    assert_eq!(capture_now(&flags).evaluate(&dependent).value(), Toggle::On);

    // An outage ages the view: the stale rule engages `halt`, so `governed`
    // serves its default and `dependent` is held, and counted, meanwhile.
    fake.set(failing(LoadCause::Unavailable));
    sleep(MAXIMUM_AGE + POLL).await;
    assert_eq!(flags.status(), SnapshotStatus::Stale);
    assert_eq!(flags.held(), 1);
    assert_eq!(
        capture_now(&flags).evaluate(&dependent).value(),
        Toggle::Off
    );
    let _ = running.shutdown().await;
}
