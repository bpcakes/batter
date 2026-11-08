#![allow(dead_code, reason = "shared by independently compiled test targets")]
//! Catalogs and processes for the presence, prune and fleet contracts.
use batter_flags_core::{
    CatalogBuilder, CivilDate, Expiry, SafeState, SettingsPrefix, StaleRule, Toggle,
    low_level::BaseBuilder,
};
use batter_flags_postgres::*;
use std::time::Duration;

/// Which catalog a build was compiled with.
#[derive(Clone, Copy)]
pub enum Build {
    /// `fast_path` (a toggle) and `halt` (a kill switch).
    Base,
    /// The base plus the `checkout` toggle.
    Extended,
}

/// A process of `build`, named `label`, with its own cadence in seconds.
pub fn process(build: Build, label: &str, liveness: u64, retention: u64) -> ProcessPresence {
    let mut catalog = CatalogBuilder::new(SettingsPrefix::new("FLAG_").unwrap());
    let serving = catalog.process_kind("serving").unwrap();
    catalog.environment_class("hosted").unwrap();
    let expiry = Expiry::on(CivilDate::new(2027, 3, 31).unwrap());
    catalog
        .release_flag("fast_path", Toggle::Off, expiry)
        .declare()
        .unwrap();
    catalog
        .kill_switch("halt", SafeState::Released, StaleRule::HoldLastKnown)
        .declare()
        .unwrap();
    if let Build::Extended = build {
        catalog
            .release_flag("checkout", Toggle::Off, expiry)
            .declare()
            .unwrap();
    }
    let base = BaseBuilder::new(catalog.build().unwrap(), serving)
        .unwrap()
        .finish();
    let labels = ProcessLabels {
        application: ProcessLabel::new("service").unwrap(),
        environment: ProcessLabel::new("hosted").unwrap(),
        build: ProcessLabel::new(label).unwrap(),
    };
    let cadence = Cadence::new(
        Duration::from_secs(liveness),
        Duration::from_secs(retention),
    )
    .unwrap();
    ProcessPresence::new(labels, &base, cadence)
}

/// A heartbeat reporting `snapshot`, holding nothing.
pub fn report(snapshot: SnapshotReport) -> PresenceReport {
    PresenceReport {
        snapshot,
        held: KeySample::default(),
        floor_engaged: KeySample::default(),
    }
}

/// Waits until a row with one-second liveness has expired on the server.
pub async fn until_silent() {
    tokio::time::sleep(Duration::from_millis(1_500)).await;
}
