//! Every kill switch reading: publication × safe state × settings literal,
//! through the one gate path every surface reads its switch through.

use batter_core::settings::SettingsSource;
use batter_flags_core::{
    CatalogBuilder, CivilDate, Expiry, KillSwitch, SafeState, SettingsPrefix, StaleRule,
    SwitchState, Toggle, Version,
    low_level::{CandidateRow, GateReading, HoldCause},
};
use tokio::time::sleep;

use super::super::{LoadCause, LoadPolicy, SnapshotStatus, source::fake::Script};
use super::support::*;
use crate::{declared, flags::gate::SwitchGate};

/// What a gate reads, by class, and the switch value behind it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Read {
    Awaiting(SwitchState),
    Open,
    Tripped,
}

fn read(flags: &crate::Flags, switch: &KillSwitch) -> Read {
    let gated = SwitchGate::single(flags, switch).unwrap().enter();
    match gated.reading {
        GateReading::AwaitingPublication(decision) => Read::Awaiting(decision.value()),
        GateReading::Open(_) => Read::Open,
        GateReading::Tripped(_) => Read::Tripped,
    }
}

const SAFE: [SafeState; 2] = [SafeState::Engaged, SafeState::Released];
const LITERALS: [Option<&str>; 3] = [None, Some("engaged"), Some("released")];

fn name(safe: SafeState, literal: Option<&str>) -> String {
    format!("{}_{}", safe.as_str(), literal.unwrap_or("absent"))
}

/// What the matrix expects for one cell.
fn expected(row: Row, safe: SafeState, literal: Option<&str>) -> Read {
    // A settings floor engages the switch whatever is or is not published.
    if literal == Some("engaged") {
        return Read::Tripped;
    }
    let safe_state = match safe {
        SafeState::Engaged => SwitchState::Engaged,
        SafeState::Released => SwitchState::Released,
    };
    match row {
        Row::NotLoaded => Read::Awaiting(safe_state),
        Row::Entry(SwitchState::Engaged) => Read::Tripped,
        Row::Entry(SwitchState::Released) => Read::Open,
        Row::Absent if safe_state.is_engaged() => Read::Tripped,
        Row::Absent => Read::Open,
    }
}

#[derive(Clone, Copy, Debug)]
enum Row {
    NotLoaded,
    Entry(SwitchState),
    Absent,
}

#[tokio::test(start_paused = true)]
async fn every_publication_safe_state_and_literal_reads_as_declared() {
    let mut catalog = CatalogBuilder::new(SettingsPrefix::new("FLAG_").unwrap());
    let serving = catalog.process_kind("serving").unwrap();
    let hosted = catalog.environment_class("hosted").unwrap();
    let mut switches = Vec::new();
    let mut settings = Vec::new();
    for safe in SAFE {
        for literal in LITERALS {
            let key = name(safe, literal);
            let switch = catalog
                .kill_switch(&key, safe, StaleRule::HoldLastKnown)
                .declare()
                .unwrap();
            if let Some(value) = literal {
                settings.push((format!("FLAG_{}", key.to_uppercase()).into(), value.into()));
            }
            switches.push((safe, literal, switch));
        }
    }
    let source = SettingsSource::from_pairs(settings).unwrap();
    let resolved = declared::resolve(
        catalog.build().unwrap(),
        &source,
        serving,
        hosted,
        CivilDate::new(2026, 11, 8).unwrap(),
    )
    .unwrap();
    let mut supervisor = supervisor();
    let (flags, fake) = register(
        &mut supervisor,
        resolved,
        LoadPolicy::start_on_defaults(timing()),
        failing(LoadCause::Unavailable),
    )
    .unwrap();
    let running = supervisor.start();
    running.status().wait_ready().await.unwrap();

    let mut cells = 0;
    for (version, row) in [
        (0, Row::NotLoaded),
        (1, Row::Entry(SwitchState::Engaged)),
        (2, Row::Entry(SwitchState::Released)),
        (3, Row::Absent),
    ] {
        let published = match row {
            Row::NotLoaded => None,
            Row::Entry(state) => Some(
                switches
                    .iter()
                    .map(|(_, _, switch)| {
                        CandidateRow::switch(switch, state, Version::new(version))
                    })
                    .collect(),
            ),
            Row::Absent => Some(Vec::new()),
        };
        if let Some(rows) = published {
            fake.set(Script::Serve {
                version: Version::new(version),
                rows,
            });
            sleep(POLL + SECOND).await;
            assert_eq!(flags.status(), SnapshotStatus::Fresh, "{row:?}");
        }
        for (safe, literal, switch) in &switches {
            assert_eq!(
                read(&flags, switch),
                expected(row, *safe, *literal),
                "{row:?} {safe:?} {literal:?}"
            );
            cells += 1;
        }
    }
    assert_eq!(cells, 24);
    let _ = running.shutdown().await;
}

#[tokio::test(start_paused = true)]
async fn a_row_needing_absent_settings_is_held_and_serves_the_base() {
    let mut catalog = CatalogBuilder::new(SettingsPrefix::new("FLAG_").unwrap());
    let serving = catalog.process_kind("serving").unwrap();
    let hosted = catalog.environment_class("hosted").unwrap();
    let outbound = catalog
        .release_flag(
            "outbound",
            Toggle::Off,
            Expiry::on(CivilDate::new(2027, 3, 31).unwrap()),
        )
        .requires_settings(Toggle::On, &["OUTBOUND_URL"])
        .declare()
        .unwrap();
    let resolved = declared::resolve(
        catalog.build().unwrap(),
        &SettingsSource::from_pairs([]).unwrap(),
        serving,
        hosted,
        CivilDate::new(2026, 11, 8).unwrap(),
    )
    .unwrap();
    let mut supervisor = supervisor();
    let at = Version::new(4);
    let script = Script::Serve {
        version: at,
        rows: vec![CandidateRow::flag(&outbound, Toggle::On, at)],
    };
    let (flags, _) = register(
        &mut supervisor,
        resolved,
        LoadPolicy::start_on_defaults(timing()),
        script,
    )
    .unwrap();
    let running = supervisor.start();
    running.status().wait_ready().await.unwrap();
    assert_eq!(flags.status(), SnapshotStatus::Fresh);
    assert_eq!(flags.held(), 1);
    let record = flags.shared.reader().record().unwrap();
    let held: Vec<_> = record
        .snapshot()
        .held(batter_flags_core::Freshness::Fresh)
        .map(|(key, cause)| (key.as_str().to_owned(), cause))
        .collect();
    assert_eq!(
        held,
        [("outbound".to_owned(), HoldCause::RequiresSettingsAbsent)]
    );
    assert_eq!(
        crate::low_level::capture_now(&flags)
            .evaluate(&outbound)
            .value(),
        Toggle::Off
    );
    let _ = running.shutdown().await;
}
