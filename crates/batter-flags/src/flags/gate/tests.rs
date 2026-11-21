//! The checked gate: what it refuses when it is built, and what it reads.
#![allow(clippy::unwrap_used, reason = "a panic fails the test")]
#![allow(clippy::missing_assert_message, reason = "test names locate failures")]

use std::time::Duration;

use batter_flags_core::{
    Catalog, CatalogBuilder, KillSwitch, Layer, PrePublication, ProcessKind, SafeState,
    SettingsPrefix, StaleRule, SwitchState, Version,
    low_level::{BaseBuilder, Candidate, CandidateRow, GateReading},
};
use tokio::time::Instant;

use super::{Flags, SwitchGate};
use crate::{GateError, reader::Reader, test::TestFlags};

/// Every kind owns `halt` and `other_halt`; only `worker` owns `export_halt`,
/// whose safe state is engaged.
struct World {
    catalog: Catalog,
    serving: ProcessKind,
    worker: ProcessKind,
    halt: KillSwitch,
    other_halt: KillSwitch,
    export_halt: KillSwitch,
}

fn world() -> World {
    let (mut builder, serving, _) =
        CatalogBuilder::new(SettingsPrefix::new("FLAG_").unwrap(), "serving", "hosted").unwrap();
    let worker = builder.process_kind("worker").unwrap();
    let halt = builder
        .kill_switch("halt", SafeState::Released, StaleRule::HoldLastKnown)
        .declare()
        .unwrap();
    let other_halt = builder
        .kill_switch("other_halt", SafeState::Released, StaleRule::HoldLastKnown)
        .declare()
        .unwrap();
    let export_halt = builder
        .kill_switch("export_halt", SafeState::Engaged, StaleRule::HoldLastKnown)
        .owned_only_by(&worker)
        .declare()
        .unwrap();
    World {
        catalog: builder.build(),
        serving,
        worker,
        halt,
        other_halt,
        export_halt,
    }
}

/// Flags that serve `engaged` published states for the given switches.
fn flags_with(
    world: World,
    kind: &str,
    engaged: &[&KillSwitch],
    released: &[&KillSwitch],
) -> Flags {
    let kind = if kind == "serving" {
        &world.serving
    } else {
        &world.worker
    };
    let mut builder = TestFlags::builder(world.catalog, kind.clone()).unwrap();
    for switch in engaged {
        builder = builder.engage(switch);
    }
    for switch in released {
        builder = builder.release(switch);
    }
    builder.build().unwrap().flags().clone()
}

/// Flags of a control plane that has published nothing, with the given floors.
fn unpublished(world: World, floors: &[&KillSwitch]) -> (Flags, crate::reader::Writer, Candidate) {
    let mut base = BaseBuilder::new(world.catalog, world.serving).unwrap();
    for floor in floors {
        base.switch(floor.key(), SwitchState::Engaged).unwrap();
    }
    let base = base.finish();
    let scope = base.validation_scope();
    let at = Version::new(5);
    let candidate =
        Candidate::new(&scope, at).row(CandidateRow::switch(&world.halt, SwitchState::Engaged, at));
    let (writer, reader) = Reader::live(
        base,
        PrePublication::ServeDefaults,
        Duration::from_secs(100),
        Instant::now(),
    );
    (Flags::new(reader), writer, candidate)
}

fn name(reading: &GateReading) -> &'static str {
    match reading {
        GateReading::AwaitingPublication(_) => "awaiting publication",
        GateReading::Open(_) => "open",
        GateReading::Tripped(_) => "tripped",
    }
}

#[test]
fn a_gate_is_built_over_switches_the_flags_can_read() {
    let w = world();
    let (halt, other, export) = (w.halt.clone(), w.other_halt.clone(), w.export_halt.clone());
    let flags = flags_with(w, "worker", &[], &[]);
    assert!(flags.gate(&halt).is_ok());
    assert!(flags.gate(&export).is_ok());
    assert!(flags.gate_any(&halt, &[&other, &export]).is_ok());
}

#[test]
fn a_switch_the_kind_does_not_own_is_refused_with_its_key() {
    let w = world();
    let export = w.export_halt.clone();
    let flags = flags_with(w, "serving", &[], &[]);
    let error = flags.gate(&export).err().unwrap();
    assert!(matches!(&error, GateError::NotOwned { key } if key.as_str() == "export_halt"));
}

#[test]
fn a_switch_from_another_catalog_is_refused_whatever_its_name() {
    let w = world();
    let foreign = world().halt;
    let flags = flags_with(w, "serving", &[], &[]);
    let error = flags.gate(&foreign).err().unwrap();
    assert!(matches!(&error, GateError::ForeignHandle { key } if key.as_str() == "halt"));
}

#[test]
fn a_switch_listed_twice_is_refused_in_any_position() {
    let w = world();
    let (halt, other) = (w.halt.clone(), w.other_halt.clone());
    let flags = flags_with(w, "serving", &[], &[]);
    for (first, rest) in [
        (&halt, vec![&halt]),
        (&halt, vec![&other, &halt]),
        (&halt, vec![&other, &other]),
    ] {
        let error = flags.gate_any(first, &rest).err().unwrap();
        assert!(matches!(error, GateError::DuplicateSwitch { .. }));
    }
    let error = flags.gate_any(&halt, &[&halt]).err().unwrap();
    assert_eq!(
        error.to_string(),
        "kill switch `halt` is listed more than once"
    );
}

#[test]
fn the_first_switch_that_fails_is_the_one_reported() {
    let w = world();
    let (halt, export) = (w.halt.clone(), w.export_halt.clone());
    let foreign = world().other_halt;
    let flags = flags_with(w, "serving", &[], &[]);
    let error = flags.gate_any(&halt, &[&foreign, &export]).err().unwrap();
    assert!(matches!(error, GateError::ForeignHandle { .. }));
    let error = flags.gate_any(&halt, &[&export, &foreign]).err().unwrap();
    assert!(matches!(error, GateError::NotOwned { .. }));
    // A duplicate is only reported for a switch that passed its own check.
    let error = flags.gate_any(&halt, &[&halt, &export]).err().unwrap();
    assert!(matches!(error, GateError::DuplicateSwitch { .. }));
}

#[test]
fn a_released_switch_reads_open_and_an_engaged_one_tripped() {
    let w = world();
    let (halt, other) = (w.halt.clone(), w.other_halt.clone());
    let flags = flags_with(w, "serving", &[&halt], &[&other]);
    assert_eq!(name(&flags.gate(&halt).unwrap().enter().reading), "tripped");
    assert_eq!(name(&flags.gate(&other).unwrap().enter().reading), "open");
}

#[test]
fn the_reading_comes_from_the_view_the_work_is_gated_under() {
    let w = world();
    let halt = w.halt.clone();
    let flags = flags_with(w, "serving", &[&halt], &[]);
    let entered = flags.gate(&halt).unwrap().enter();
    // One capture: the reading's own source is the returned view's source, and
    // evaluating the switch again on that view agrees with the reading.
    assert_eq!(entered.reading.decision().source(), entered.view.source());
    assert_eq!(
        entered.view.switch(&halt).value(),
        entered.reading.decision().value()
    );
}

#[test]
fn any_tripped_switch_decides_whichever_position_it_is_in() {
    let w = world();
    let (halt, other) = (w.halt.clone(), w.other_halt.clone());
    let flags = flags_with(w, "serving", &[&other], &[&halt]);
    for (first, rest) in [(&halt, &other), (&other, &halt)] {
        let gate: SwitchGate = flags.gate_any(first, &[rest]).unwrap();
        let entered = gate.enter();
        assert_eq!(name(&entered.reading), "tripped");
        assert_eq!(entered.reading.decision().value(), SwitchState::Engaged);
    }

    // With none tripped, the gate is open, whichever switch is listed first.
    let w = world();
    let (halt, other) = (w.halt.clone(), w.other_halt.clone());
    let flags = flags_with(w, "serving", &[], &[&halt, &other]);
    for (first, rest) in [(&halt, &other), (&other, &halt)] {
        let entered = flags.gate_any(first, &[rest]).unwrap().enter();
        assert_eq!(name(&entered.reading), "open");
    }
}

#[test]
fn before_any_publication_the_gate_waits_and_a_floor_still_trips() {
    let w = world();
    let (halt, other) = (w.halt.clone(), w.other_halt.clone());
    let (flags, _writer, _snapshot) = unpublished(w, &[]);
    let waiting = flags.gate_any(&halt, &[&other]).unwrap().enter();
    assert_eq!(name(&waiting.reading), "awaiting publication");

    // A settings floor is the operator's explicit setting, so it trips even
    // before anything is published, and it decides over a switch still waiting.
    let w = world();
    let (halt, other) = (w.halt.clone(), w.other_halt.clone());
    let (flags, _writer, _snapshot) = unpublished(w, &[&other]);
    for (first, rest) in [(&halt, &other), (&other, &halt)] {
        let entered = flags.gate_any(first, &[rest]).unwrap().enter();
        assert_eq!(name(&entered.reading), "tripped");
        assert_eq!(entered.reading.decision().layer(), &Layer::Floor);
    }
}

#[test]
fn each_entry_is_one_capture_and_a_later_publication_reaches_the_next_entry_only() {
    let w = world();
    let halt = w.halt.clone();
    let (flags, mut writer, candidate) = unpublished(w, &[]);
    let gate = flags.gate(&halt).unwrap();

    let before = gate.enter();
    assert_eq!(name(&before.reading), "awaiting publication");

    writer.read_at(candidate, Instant::now());
    let after = gate.enter();
    assert_eq!(name(&after.reading), "tripped");
    // The earlier entry's view is frozen: it still says nothing was published.
    assert_eq!(before.view.switch(&halt).layer(), &Layer::Base);
    assert!(matches!(
        before.view.source(),
        batter_flags_core::Source::NotLoaded { .. }
    ));
}
