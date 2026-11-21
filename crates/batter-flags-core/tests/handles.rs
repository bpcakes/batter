//! Handles: a handle from another catalog or a kind that does not own the
//! flag yields its own default with a counted error, and the base, snapshot and
//! view of one catalog never serve another's.
#![allow(clippy::unwrap_used, reason = "a panic fails the test")]
#![allow(clippy::missing_assert_message, reason = "test names locate failures")]

#[path = "support/fixtures.rs"]
mod fixtures;
#[path = "support/world.rs"]
mod world;

use batter_flags_core::{
    ErrorCause, Flag, Freshness, Layer, Reason, SwitchState, Toggle, low_level::Candidate,
};
use fixtures::Checkout;
use world::{Literal, base, flag_row, snapshot, world};

/// Row: a foreign or not-owned handle.
#[test]
fn a_foreign_or_not_owned_handle_yields_its_default_with_an_error() {
    let (catalog, h) = world();
    let (_other_catalog, other) = world();
    let base = base(
        catalog,
        &h.serving,
        &[Literal::Flag("checkout_style", "express")],
    );
    let snapshot = snapshot(
        &base,
        2,
        [
            flag_row(&h.checkout_style, Checkout::Express, 2),
            flag_row(&h.fast_path, Toggle::On, 2),
        ],
    );
    let view = base
        .control_plane_view(&snapshot, Freshness::Fresh)
        .unwrap();

    let foreign = view.evaluate(&other.fast_path);
    assert_eq!(
        foreign.value(),
        Toggle::Off,
        "the handle's own code default"
    );
    assert_eq!(foreign.reason(), Reason::Error);
    assert_eq!(foreign.error(), Some(ErrorCause::ForeignCatalog));
    assert_eq!(foreign.layer(), &Layer::Base);
    assert_eq!(foreign.source(), view.source());

    let foreign_style = view.evaluate(&other.checkout_style);
    assert_eq!(foreign_style.value(), Checkout::Classic);
    let foreign_switch = view.switch(&other.export_halt);
    assert_eq!(
        (foreign_switch.value(), foreign_switch.error()),
        (SwitchState::Engaged, Some(ErrorCause::ForeignCatalog)),
        "a foreign switch yields its own safe state"
    );

    // `worker_mode` is owned by `worker` alone, and `export_halt` likewise.
    let not_owned = view.evaluate(&h.worker_mode);
    assert_eq!(not_owned.value(), Toggle::Off);
    assert_eq!(not_owned.error(), Some(ErrorCause::NotOwned));
    let not_owned_switch = view.switch(&h.export_halt);
    assert_eq!(
        (not_owned_switch.value(), not_owned_switch.error()),
        (SwitchState::Engaged, Some(ErrorCause::NotOwned))
    );
    // A kind that only replays pins of `new_checkout` cannot evaluate it live.
    assert_eq!(base.error_count(ErrorCause::ForeignCatalog), 3);
    assert_eq!(base.error_count(ErrorCause::NotOwned), 2);

    // The owned handles still answer.
    assert_eq!(view.evaluate(&h.checkout_style).value(), Checkout::Express);
}

#[test]
fn a_pinning_kind_never_evaluates_the_flag_live() {
    let (catalog, h) = world();
    let base = base(catalog, &h.worker, &[]);
    let view = base.declared_view();
    let decision = view.evaluate(&h.new_checkout);
    assert_eq!(decision.error(), Some(ErrorCause::NotOwned));
    assert_eq!(decision.value(), Toggle::Off);
}

#[test]
fn two_catalogs_in_one_process_never_read_each_others_handles() {
    let (first_catalog, first) = world();
    let (second_catalog, second) = world();
    let first_base = base(
        first_catalog,
        &first.serving,
        &[Literal::Flag("fast_path", "on")],
    );
    let second_base = base(second_catalog, &second.serving, &[]);
    let first_view = first_base.declared_view();
    let second_view = second_base.declared_view();
    assert_eq!(first_view.evaluate(&first.fast_path).value(), Toggle::On);
    assert_eq!(second_view.evaluate(&second.fast_path).value(), Toggle::Off);
    assert_eq!(
        first_view.evaluate(&second.fast_path).error(),
        Some(ErrorCause::ForeignCatalog)
    );
    assert_eq!(
        second_view.evaluate(&first.fast_path).error(),
        Some(ErrorCause::ForeignCatalog)
    );
    // The same declarations, built twice, have the same digests and different handles.
    assert_eq!(
        first_base.catalog().digest(),
        second_base.catalog().digest()
    );
    assert_ne!(first.fast_path, second.fast_path);
}

#[test]
fn a_snapshot_or_view_for_another_base_is_refused() {
    let (first_catalog, first) = world();
    let (second_catalog, second) = world();
    let first_base = base(first_catalog, &first.serving, &[]);
    let second_base = base(second_catalog, &second.serving, &[]);
    let foreign = snapshot(&second_base, 1, []);
    assert!(
        first_base
            .control_plane_view(&foreign, Freshness::Fresh)
            .is_err()
    );
    assert!(
        first_base
            .persisted_file_view(&foreign, Freshness::Fresh)
            .is_err()
    );
    assert!(first_base.test_view(&foreign, Freshness::Fresh).is_err());
    let scope = first_base.validation_scope();
    assert!(
        Candidate::new(&scope, batter_flags_core::Version::new(1))
            .carrying(&foreign)
            .is_err()
    );
}

#[test]
fn typed_values_round_trip_through_every_layer() {
    let (catalog, h) = world();
    let base = base(catalog, &h.serving, &[]);
    let snapshot = snapshot(
        &base,
        3,
        [flag_row(&h.checkout_style, Checkout::Express, 3)],
    );
    let view = base
        .control_plane_view(&snapshot, Freshness::Fresh)
        .unwrap();
    let decision = view.evaluate(&h.checkout_style);
    assert_eq!(decision.value(), Checkout::Express);
    assert_eq!(decision.variant(), "express");
    assert_eq!(h.checkout_style.code_default(), Checkout::Classic);
}
