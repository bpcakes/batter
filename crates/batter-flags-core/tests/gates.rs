//! What a gate reads from its switch: waiting for a first publication is not
//! the same as an operator tripping the switch.
#![allow(clippy::unwrap_used, reason = "a panic fails the test")]
#![allow(clippy::missing_assert_message, reason = "test names locate failures")]

#[path = "support/capture.rs"]
mod capture;
#[path = "support/fixtures.rs"]
mod fixtures;
#[path = "../../../test-support/dispatch.rs"]
mod test_dispatch;
#[path = "support/world.rs"]
mod world;

use batter_flags_core::{
    ErrorCause, Freshness, Layer, PrePublication, Reason, SwitchState,
    low_level::{GateReading, HandleRefusal, gate_switch},
};
use capture::Capture;
use tracing::Level;
use world::{Literal, base, file_snapshot, switch_row, world};

const POLICIES: [PrePublication; 2] = [PrePublication::ServeDefaults, PrePublication::Unready];
const FRESHNESS: [Freshness; 2] = [Freshness::Fresh, Freshness::Stale];

/// A match with no wildcard: a fourth reading would not compile until every
/// gate decided what to do with it.
fn name(reading: &GateReading) -> &'static str {
    match reading {
        GateReading::AwaitingPublication(_) => "awaiting publication",
        GateReading::Open(_) => "open",
        GateReading::Tripped(_) => "tripped",
    }
}

#[test]
fn before_any_publication_a_gate_waits_whatever_the_switch_would_serve() {
    let (catalog, h) = world();
    // `checkout_halt` is released when unpublished and holds its last state;
    // `export_halt` is engaged when unpublished and engages when stale.
    let base = base(catalog, &h.worker, &[]);
    for policy in POLICIES {
        for freshness in FRESHNESS {
            let view = base.not_loaded_view(policy, freshness);
            for switch in [&h.checkout_halt, &h.export_halt] {
                let reading = gate_switch(&view, switch);
                let label = format!("{policy:?}, {freshness:?}, {}", switch.key());
                assert_eq!(name(&reading), "awaiting publication", "{label}");
            }
        }
    }
    // A decision that reads as engaged can still be a wait: the switch's own
    // safe state, or its stale rule, is not an operator's decision.
    let stale = base.not_loaded_view(PrePublication::ServeDefaults, Freshness::Stale);
    let engaged_by_rule = gate_switch(&stale, &h.export_halt);
    assert_eq!(engaged_by_rule.decision().value(), SwitchState::Engaged);
    assert_eq!(engaged_by_rule.decision().layer(), &Layer::StaleRule);
    assert!(matches!(
        engaged_by_rule,
        GateReading::AwaitingPublication(_)
    ));
}

#[test]
fn a_settings_floor_is_the_operators_trip_even_before_publication() {
    let (catalog, h) = world();
    let base = base(
        catalog,
        &h.worker,
        &[Literal::Switch("checkout_halt", SwitchState::Engaged)],
    );
    for policy in POLICIES {
        let view = base.not_loaded_view(policy, Freshness::Fresh);
        let reading = gate_switch(&view, &h.checkout_halt);
        assert!(matches!(&reading, GateReading::Tripped(_)), "{policy:?}");
        assert_eq!(reading.decision().layer(), &Layer::Floor);
        assert_eq!(reading.decision().reason(), Reason::Static);
        // The switch with no floor is still waiting.
        let other = gate_switch(&view, &h.export_halt);
        assert!(matches!(other, GateReading::AwaitingPublication(_)));
    }
}

#[test]
fn a_persisted_file_is_a_publication() {
    let (catalog, h) = world();
    let base = base(catalog, &h.worker, &[]);
    let engaged = file_snapshot(
        &base,
        7,
        [
            switch_row(&h.checkout_halt, SwitchState::Engaged, 6),
            switch_row(&h.export_halt, SwitchState::Released, 6),
        ],
    );
    let fresh = base
        .persisted_file_view(&engaged, Freshness::Fresh)
        .unwrap();
    assert!(matches!(
        gate_switch(&fresh, &h.checkout_halt),
        GateReading::Tripped(_)
    ));
    assert!(matches!(
        gate_switch(&fresh, &h.export_halt),
        GateReading::Open(_)
    ));
    // Stale, the file's `EngageWhenStale` switch trips by its rule.
    let stale = base
        .persisted_file_view(&engaged, Freshness::Stale)
        .unwrap();
    let reading = gate_switch(&stale, &h.export_halt);
    assert!(matches!(&reading, GateReading::Tripped(_)));
    assert_eq!(reading.decision().layer(), &Layer::StaleRule);
}

#[test]
fn declared_mode_has_no_waiting_window() {
    let (catalog, h) = world();
    let base = base(catalog, &h.worker, &[]);
    let view = base.declared_view();
    // Declared mode serves each switch's safe state: released and engaged.
    assert_eq!(name(&gate_switch(&view, &h.checkout_halt)), "open");
    assert_eq!(name(&gate_switch(&view, &h.export_halt)), "tripped");
}

/// The low-level read fails closed: a switch this base cannot answer for is
/// tripped whatever its safe state, and the decision carries the safe state
/// and the cause. Gates built by `batter-flags` check the switch first, so
/// they never reach this.
#[test]
fn a_switch_the_base_cannot_answer_for_reads_as_tripped_with_an_error() {
    let (catalog, h) = world();
    let (_other_catalog, other) = world();
    let base = base(catalog, &h.serving, &[]);
    let view = base.declared_view();

    // `export_halt` belongs to `worker`: its safe state is engaged.
    let not_owned = gate_switch(&view, &h.export_halt);
    assert!(matches!(&not_owned, GateReading::Tripped(_)));
    assert_eq!(not_owned.decision().error(), Some(ErrorCause::NotOwned));
    // Another catalog's `checkout_halt`: its safe state is released, and it
    // used to read as open, so the gated work ran.
    let foreign = gate_switch(&view, &other.checkout_halt);
    assert!(matches!(&foreign, GateReading::Tripped(_)));
    assert_eq!(foreign.decision().value(), SwitchState::Released);
    assert_eq!(foreign.decision().error(), Some(ErrorCause::ForeignCatalog));
    // Before publication too: unanswerable outranks awaiting.
    let unpublished = base.not_loaded_view(PrePublication::ServeDefaults, Freshness::Fresh);
    assert!(matches!(
        gate_switch(&unpublished, &other.checkout_halt),
        GateReading::Tripped(_)
    ));

    // The check that a gate constructor makes refuses both.
    assert_eq!(
        base.check_switch(&h.export_halt),
        Err(HandleRefusal::NotOwned)
    );
    assert_eq!(
        base.check_switch(&other.checkout_halt),
        Err(HandleRefusal::ForeignCatalog)
    );
}

#[test]
fn a_gate_read_emits_one_event_whatever_it_reads() {
    let capture = Capture::new();
    let (catalog, h) = world();
    let base = base(catalog, &h.worker, &[]);
    let views = capture.within(|| {
        [
            base.not_loaded_view(PrePublication::ServeDefaults, Freshness::Stale),
            base.declared_view(),
        ]
    });
    for view in &views {
        for switch in [&h.checkout_halt, &h.export_halt] {
            capture.clear();
            let reading = gate_switch(view, switch);
            let events = capture.events();
            assert_eq!(events.len(), 1, "{}", name(&reading));
            let engaged = reading.decision().value().is_engaged();
            assert_eq!(
                events[0].level,
                if engaged { Level::INFO } else { Level::DEBUG },
                "the level follows the switch's value, whatever the reading"
            );
        }
    }
}
