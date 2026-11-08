//! A catalog, a scripted store and a supervisor for the loader contracts.

use std::{sync::Arc, time::Duration};

use batter_core::{
    cleanup::CleanupBudget,
    lifecycle::{ShutdownBudget, Supervisor},
    settings::SettingsSource,
};
use batter_flags_core::{
    CatalogBuilder, CivilDate, Expiry, KillSwitch, ReleaseFlag, SafeState, SettingsPrefix,
    StaleRule, SwitchState, Toggle, Version, low_level::CandidateRow,
};
use batter_flags_postgres::{PooledStore, StoreTiming};

use super::super::{
    ApplicationName, BuildId, HeartbeatCadence, LoadPolicy, Presence, RefreshTiming, RegisterError,
    RegisteredFlags, register_with,
    source::{
        FetchFailure, LoadSource,
        fake::{Fake, Script},
    },
};
use crate::declared::{self, Resolved};

pub(super) const SECOND: Duration = Duration::from_secs(1);
/// Poll every 5 s; views stay fresh 10 s. The store's timing gives a 4 s
/// refresh budget, so 9 s is the shortest maximum age.
pub(super) const POLL: Duration = Duration::from_secs(5);
pub(super) const MAXIMUM_AGE: Duration = Duration::from_secs(10);

pub(super) struct Handles {
    pub(super) flag: ReleaseFlag<Toggle>,
    pub(super) halt: KillSwitch,
    pub(super) stale_halt: KillSwitch,
}

/// A catalog with a toggle, a held-last-known switch and an engage-when-stale
/// switch, resolved against `settings`.
pub(super) fn resolved(settings: &[(&str, &str)]) -> (Resolved, Handles) {
    let mut catalog = CatalogBuilder::new(SettingsPrefix::new("FLAG_").unwrap());
    let serving = catalog.process_kind("serving").unwrap();
    let hosted = catalog.environment_class("hosted").unwrap();
    let flag = catalog
        .release_flag(
            "fast_path",
            Toggle::Off,
            Expiry::on(CivilDate::new(2027, 3, 31).unwrap()),
        )
        .declare()
        .unwrap();
    let halt = catalog
        .kill_switch("halt", SafeState::Released, StaleRule::HoldLastKnown)
        .declare()
        .unwrap();
    let stale_halt = catalog
        .kill_switch(
            "stale_halt",
            SafeState::Released,
            StaleRule::EngageWhenStale,
        )
        .declare()
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
    (
        resolved,
        Handles {
            flag,
            halt,
            stale_halt,
        },
    )
}

/// A store no heartbeat or prune can reach.
pub(super) fn unreachable_store() -> PooledStore {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect_lazy("postgres://flags@127.0.0.1:1/flags")
        .unwrap();
    PooledStore::new(pool, store_timing())
}

pub(super) fn store_timing() -> StoreTiming {
    StoreTiming::new(SECOND, SECOND, SECOND, SECOND).unwrap()
}

pub(super) fn timing() -> RefreshTiming {
    RefreshTiming::new(POLL, MAXIMUM_AGE).unwrap()
}

/// With a last-good file the run budget grows by the 2 s file sub-deadline,
/// so the shortest maximum age is 11 s.
pub(super) const FILE_MAXIMUM_AGE: Duration = Duration::from_secs(12);

pub(super) fn file_timing() -> RefreshTiming {
    RefreshTiming::new(POLL, FILE_MAXIMUM_AGE).unwrap()
}

pub(super) fn presence() -> Presence {
    Presence::new(
        ApplicationName::new("service").unwrap(),
        BuildId::new("1.0.0+test").unwrap(),
        HeartbeatCadence::new(Duration::from_secs(5), Duration::from_secs(60)).unwrap(),
    )
}

pub(super) fn supervisor() -> Supervisor {
    Supervisor::new(
        ShutdownBudget::new(
            SECOND * 5,
            SECOND * 5,
            SECOND * 5,
            CleanupBudget::new(SECOND * 5, SECOND * 5, SECOND * 5).unwrap(),
        )
        .unwrap(),
    )
}

/// Registers the loader over a scripted store.
pub(super) fn register(
    supervisor: &mut Supervisor,
    resolved: Resolved,
    policy: LoadPolicy,
    script: Script,
) -> Result<(RegisteredFlags, Arc<Fake>), RegisterError> {
    let fake = Arc::new(Fake::new(script));
    let flags = register_with(
        supervisor,
        LoadSource::Fake(fake.clone()),
        &unreachable_store(),
        resolved,
        policy,
        presence(),
    )?;
    Ok((flags, fake))
}

/// The store at `version`, with the switch and the toggle as given.
pub(super) fn serve(handles: &Handles, version: u64, halt: SwitchState, flag: Toggle) -> Script {
    let at = Version::new(version);
    Script::Serve {
        version: at,
        rows: vec![
            CandidateRow::switch(&handles.halt, halt, at),
            CandidateRow::flag(&handles.flag, flag, at),
        ],
    }
}

pub(super) fn failing(cause: super::super::LoadCause) -> Script {
    Script::Fail(FetchFailure::Failed(cause))
}

pub(super) fn schema_behind(found: i32) -> Script {
    Script::Fail(FetchFailure::SchemaBehind { found })
}
