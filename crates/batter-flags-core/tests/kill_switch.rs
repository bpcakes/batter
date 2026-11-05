//! The kill-switch function, every cell: mode × floor × freshness × stale rule
//! × entry state × safe state, against an oracle written from the precedence
//! rule (floor, stale rule, served state) and not from the implementation.
#![allow(clippy::unwrap_used, reason = "a panic fails the test")]
#![allow(clippy::missing_assert_message, reason = "test names locate failures")]

#[path = "support/fixtures.rs"]
mod fixtures;
#[path = "support/world.rs"]
mod world;

use std::sync::Arc;

use batter_flags_core::{
    Catalog, CatalogBuilder, ErrorCause, Freshness, KillSwitch, Layer, PrePublication, ProcessKind,
    Reason, SafeState, SettingsPrefix, SnapshotView, Source, StaleRule, SwitchState,
    low_level::{Base, Candidate, GateReading, Provenance, Snapshot, gate_switch},
};
use world::{Literal, base, snapshot, switch_row, undecodable_row, version};

/// One switch per (safe state, stale rule) pair.
struct Declared {
    name: &'static str,
    safe: SafeState,
    rule: StaleRule,
}

const DECLARED: [Declared; 4] = [
    Declared {
        name: "released_hold",
        safe: SafeState::Released,
        rule: StaleRule::HoldLastKnown,
    },
    Declared {
        name: "released_engage",
        safe: SafeState::Released,
        rule: StaleRule::EngageWhenStale,
    },
    Declared {
        name: "engaged_hold",
        safe: SafeState::Engaged,
        rule: StaleRule::HoldLastKnown,
    },
    Declared {
        name: "engaged_engage",
        safe: SafeState::Engaged,
        rule: StaleRule::EngageWhenStale,
    },
];

/// A catalog of the four switches, which every process kind owns.
fn switches() -> (Catalog, ProcessKind, Vec<KillSwitch>) {
    let mut builder = CatalogBuilder::new(SettingsPrefix::new("FLAG_").unwrap());
    let serving = builder.process_kind("serving").unwrap();
    builder.environment_class("hosted").unwrap();
    let handles = DECLARED
        .iter()
        .map(|switch| {
            builder
                .kill_switch(switch.name, switch.safe, switch.rule)
                .declare()
                .unwrap()
        })
        .collect();
    (builder.build().unwrap(), serving, handles)
}

/// What the process's settings supplied for every switch.
#[derive(Clone, Copy, Debug)]
enum Floor {
    Absent,
    Released,
    Engaged,
}

const FLOORS: [Floor; 3] = [Floor::Absent, Floor::Released, Floor::Engaged];

impl Floor {
    fn literals(self) -> Vec<Literal<'static>> {
        let state = match self {
            Self::Absent => return Vec::new(),
            Self::Released => SwitchState::Released,
            Self::Engaged => SwitchState::Engaged,
        };
        DECLARED
            .iter()
            .map(|switch| Literal::Switch(switch.name, state))
            .collect()
    }
}

/// The state of every switch's entry in the snapshot a view serves.
#[derive(Clone, Copy, Debug)]
enum Row {
    Absent,
    Published(SwitchState),
    /// Held after a published state, for this many reads running.
    HeldAfter(SwitchState, u64),
    /// Held with nothing last published.
    HeldFresh,
}

const ROWS: [Row; 8] = [
    Row::Absent,
    Row::Published(SwitchState::Engaged),
    Row::Published(SwitchState::Released),
    Row::HeldAfter(SwitchState::Engaged, 1),
    Row::HeldAfter(SwitchState::Released, 1),
    Row::HeldAfter(SwitchState::Engaged, 2),
    Row::HeldAfter(SwitchState::Released, 2),
    Row::HeldFresh,
];

/// The version a published state was served at; the row itself was last
/// written at 5, so a layer reporting 5 would be reporting the wrong thing.
const PUBLISHED_AT: u64 = 10;
const WRITTEN_AT: u64 = 5;

fn held_rows(at: u64) -> impl Iterator<Item = batter_flags_core::low_level::CandidateRow> {
    DECLARED
        .iter()
        .map(move |switch| undecodable_row(switch.name, "switch", "switch", at))
}

/// The snapshot a row case is, read from `provenance`.
fn snapshot_for(
    base: &Base,
    handles: &[KillSwitch],
    row: Row,
    provenance: Provenance,
) -> Arc<Snapshot> {
    let scope = base.validation_scope();
    let candidate = || match provenance {
        Provenance::ControlPlane => Candidate::new(&scope),
        Provenance::LastGoodFile => Candidate::from_last_good_file(&scope),
    };
    let validate = |candidate: Candidate| Arc::new(Snapshot::validate(&scope, candidate).unwrap());
    let rows = |state| {
        handles
            .iter()
            .map(move |handle| switch_row(handle, state, WRITTEN_AT))
    };
    match row {
        Row::Absent => validate(candidate().at(version(20))),
        Row::Published(state) => validate(candidate().at(version(PUBLISHED_AT)).rows(rows(state))),
        Row::HeldFresh => validate(candidate().at(version(12)).rows(held_rows(12))),
        Row::HeldAfter(state, reads) => {
            let mut current = validate(candidate().at(version(PUBLISHED_AT)).rows(rows(state)));
            for read in 0..reads {
                let at = 12 + 3 * read;
                current = validate(
                    candidate()
                        .at(version(at))
                        .rows(held_rows(at))
                        .carrying(&current)
                        .unwrap(),
                );
            }
            current
        }
    }
}

type Want = (SwitchState, Layer, Reason, Option<ErrorCause>);

/// The precedence rule: floor, then the stale rule, then the served state.
/// How a loaded view came by its snapshot.
#[derive(Clone, Copy, Debug)]
enum Loaded {
    ControlPlane,
    PersistedFile,
    Test,
}

fn loaded(floor: Floor, freshness: Freshness, source: Loaded, switch: &Declared, row: Row) -> Want {
    let stale = freshness == Freshness::Stale;
    let file = matches!(source, Loaded::PersistedFile);
    if matches!(floor, Floor::Engaged) {
        return (SwitchState::Engaged, Layer::Floor, Reason::Static, None);
    }
    if stale && switch.rule == StaleRule::EngageWhenStale {
        return (SwitchState::Engaged, Layer::StaleRule, Reason::Stale, None);
    }
    match row {
        Row::Published(state) => {
            let reason = if stale || file {
                Reason::Stale
            } else {
                Reason::Static
            };
            (state, Layer::Entry, reason, None)
        }
        Row::HeldAfter(state, _) => (
            state,
            Layer::Held {
                since: version(PUBLISHED_AT),
            },
            Reason::Stale,
            None,
        ),
        Row::Absent | Row::HeldFresh => (switch.safe.into(), Layer::Base, Reason::Default, None),
    }
}

fn not_loaded(
    floor: Floor,
    freshness: Freshness,
    policy: PrePublication,
    switch: &Declared,
) -> Want {
    let served = loaded(floor, freshness, Loaded::Test, switch, Row::Absent);
    match (served.1 == Layer::Base, policy) {
        (true, PrePublication::Unready) => (
            served.0,
            Layer::Base,
            Reason::Error,
            Some(ErrorCause::ProviderNotReady),
        ),
        _ => served,
    }
}

fn declared(floor: Floor, switch: &Declared) -> Want {
    match floor {
        Floor::Absent => (switch.safe.into(), Layer::Base, Reason::Default, None),
        Floor::Released => (SwitchState::Released, Layer::Entry, Reason::Static, None),
        Floor::Engaged => (SwitchState::Engaged, Layer::Entry, Reason::Static, None),
    }
}

fn check(view: &SnapshotView, handle: &KillSwitch, want: &Want, label: &str) {
    let decision = view.switch(handle);
    let got: Want = (
        decision.value(),
        decision.layer().clone(),
        decision.reason(),
        decision.error(),
    );
    assert_eq!(&got, want, "{label}");
    assert_eq!(decision.variant(), decision.value().as_str(), "{label}");
    assert_eq!(decision.source(), view.source(), "{label}");
    let reading = gate_switch(view, handle);
    assert_eq!(
        reading.decision(),
        &decision,
        "{label}: a gate reads the same decision as the view"
    );
    assert_eq!(
        reading_name(&reading),
        expected_reading(view.source(), want),
        "{label}"
    );
}

/// What a gate does with each cell: nothing published is waiting, unless the
/// settings floor tripped it; otherwise the switch's state decides.
fn expected_reading(source: Source, want: &Want) -> &'static str {
    let (state, layer, ..) = want;
    if matches!(source, Source::NotLoaded { .. }) && *layer != Layer::Floor {
        "awaiting"
    } else if state.is_engaged() {
        "tripped"
    } else {
        "open"
    }
}

fn reading_name(reading: &GateReading) -> &'static str {
    match reading {
        GateReading::AwaitingPublication(_) => "awaiting",
        GateReading::Open(_) => "open",
        GateReading::Tripped(_) => "tripped",
    }
}

const FRESHNESS: [Freshness; 2] = [Freshness::Fresh, Freshness::Stale];
const POLICIES: [PrePublication; 2] = [PrePublication::ServeDefaults, PrePublication::Unready];
const SOURCES: [Loaded; 3] = [Loaded::ControlPlane, Loaded::PersistedFile, Loaded::Test];

/// Declared mode: the settings literal, else the safe state.
fn declared_cells(base: &Base, floor: Floor, handles: &[KillSwitch]) -> usize {
    let view = base.declared_view();
    for (switch, handle) in DECLARED.iter().zip(handles) {
        let label = format!("declared, floor {floor:?}, {}", switch.name);
        check(&view, handle, &declared(floor, switch), &label);
    }
    DECLARED.len()
}

/// Before the first publication, under each load policy and freshness.
fn not_loaded_cells(base: &Base, floor: Floor, handles: &[KillSwitch]) -> usize {
    let mut cells = 0;
    for freshness in FRESHNESS {
        for policy in POLICIES {
            let view = base.not_loaded_view(policy, freshness);
            for (switch, handle) in DECLARED.iter().zip(handles) {
                let label = format!(
                    "not loaded {policy:?}, floor {floor:?}, {freshness:?}, {}",
                    switch.name
                );
                check(
                    &view,
                    handle,
                    &not_loaded(floor, freshness, policy, switch),
                    &label,
                );
                cells += 1;
            }
        }
    }
    cells
}

/// Over one snapshot, under each source and freshness.
fn loaded_cells(
    base: &Base,
    floor: Floor,
    handles: &[KillSwitch],
    (row, from_control_plane, from_file): (Row, &Arc<Snapshot>, &Arc<Snapshot>),
) -> usize {
    let mut cells = 0;
    for freshness in FRESHNESS {
        for source in SOURCES {
            let view = match source {
                Loaded::ControlPlane => base.control_plane_view(from_control_plane, freshness),
                Loaded::PersistedFile => base.persisted_file_view(from_file, freshness),
                Loaded::Test => base.test_view(from_control_plane, freshness),
            }
            .unwrap();
            for (switch, handle) in DECLARED.iter().zip(handles) {
                let label = format!(
                    "{source:?}, floor {floor:?}, {freshness:?}, {row:?}, {}",
                    switch.name
                );
                let want = loaded(floor, freshness, source, switch, row);
                check(&view, handle, &want, &label);
                cells += 1;
            }
        }
    }
    cells
}

#[test]
fn every_cell_of_the_kill_switch_function() {
    let mut cells = 0;
    for floor in FLOORS {
        let (catalog, serving, handles) = switches();
        let base = base(catalog, &serving, &floor.literals());
        cells += declared_cells(&base, floor, &handles);
        cells += not_loaded_cells(&base, floor, &handles);
        for row in ROWS {
            let published = snapshot_for(&base, &handles, row, Provenance::ControlPlane);
            let restored = snapshot_for(&base, &handles, row, Provenance::LastGoodFile);
            cells += loaded_cells(&base, floor, &handles, (row, &published, &restored));
        }
    }
    // declared 3×4, not loaded 3×2×2×4, loaded 3×8×2×3×4: no loop shrank silently.
    assert_eq!(cells, 12 + 48 + 576);
}

/// A published `released` releases a `SafeState::Engaged` switch; nothing
/// releases a floor; staleness is not sticky once a read confirms the snapshot.
#[test]
fn rows_release_a_safe_state_but_never_a_floor_and_staleness_is_not_sticky() {
    let (catalog, serving, handles) = switches();
    let base = base(catalog, &serving, &[]);
    let released = snapshot_for(
        &base,
        &handles,
        Row::Published(SwitchState::Released),
        Provenance::ControlPlane,
    );
    let engaged_hold = &handles[2];
    let engaged_engage = &handles[3];

    let fresh = base
        .control_plane_view(&released, Freshness::Fresh)
        .unwrap();
    assert_eq!(fresh.switch(engaged_hold).value(), SwitchState::Released);
    assert_eq!(fresh.switch(engaged_engage).value(), SwitchState::Released);

    let stale = base
        .control_plane_view(&released, Freshness::Stale)
        .unwrap();
    assert_eq!(stale.switch(engaged_hold).value(), SwitchState::Released);
    assert_eq!(stale.switch(engaged_engage).value(), SwitchState::Engaged);

    let again = base
        .control_plane_view(&released, Freshness::Fresh)
        .unwrap();
    assert_eq!(
        again.switch(engaged_engage).value(),
        SwitchState::Released,
        "a view captured fresh over the same snapshot releases the switch again"
    );
}

/// Holding keeps the state last served, whichever state it was, and a later
/// published row replaces what was carried.
#[test]
fn holding_keeps_the_last_state_and_a_later_row_replaces_it() {
    let (catalog, serving, handles) = switches();
    let base = base(catalog, &serving, &[]);
    let scope = base.validation_scope();
    let engaged_hold = &handles[2];
    let view_of =
        |snapshot: &Arc<Snapshot>| base.control_plane_view(snapshot, Freshness::Fresh).unwrap();

    let engaged = snapshot(
        &base,
        10,
        [switch_row(engaged_hold, SwitchState::Engaged, 5)],
    );
    let held = Arc::new(
        Snapshot::validate(
            &scope,
            Candidate::new(&scope)
                .at(version(11))
                .row(undecodable_row("engaged_hold", "switch", "switch", 11))
                .carrying(&engaged)
                .unwrap(),
        )
        .unwrap(),
    );
    let decision = view_of(&held).switch(engaged_hold);
    assert_eq!(decision.value(), SwitchState::Engaged);
    assert_eq!(decision.layer(), &Layer::Held { since: version(10) });

    // A published row replaces what was carried, and a later hold carries that.
    let released = Arc::new(
        Snapshot::validate(
            &scope,
            Candidate::new(&scope)
                .at(version(12))
                .row(switch_row(engaged_hold, SwitchState::Released, 12))
                .carrying(&held)
                .unwrap(),
        )
        .unwrap(),
    );
    assert_eq!(
        view_of(&released).switch(engaged_hold).value(),
        SwitchState::Released
    );
    let held_again = Arc::new(
        Snapshot::validate(
            &scope,
            Candidate::new(&scope)
                .at(version(13))
                .row(undecodable_row("engaged_hold", "switch", "switch", 13))
                .carrying(&released)
                .unwrap(),
        )
        .unwrap(),
    );
    let decision = view_of(&held_again).switch(engaged_hold);
    assert_eq!(
        (decision.value(), decision.layer()),
        (SwitchState::Released, &Layer::Held { since: version(12) }),
        "the hold keeps `released` even though the safe state is engaged"
    );

    // A candidate that does not carry forgets: the safe state serves.
    let forgotten = snapshot(
        &base,
        14,
        [undecodable_row("engaged_hold", "switch", "switch", 14)],
    );
    assert_eq!(
        view_of(&forgotten).switch(engaged_hold).value(),
        SwitchState::Engaged
    );
}
