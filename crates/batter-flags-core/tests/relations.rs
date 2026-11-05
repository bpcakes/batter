//! Relationships between declared flags are stated on the catalog, so a pair
//! can be made reciprocal whichever flag was declared first.
#![allow(clippy::unwrap_used, reason = "a panic fails the test")]
#![allow(clippy::missing_assert_message, reason = "test names locate failures")]

#[path = "support/fixtures.rs"]
mod fixtures;

use batter_flags_core::{
    BuildError, Catalog, CatalogBuilder, Flag, InvariantRule, ModeFlag, Toggle,
};
use fixtures::{Checkout, vocabulary_builder};

fn flag(builder: &mut CatalogBuilder, key: &str, default: Toggle) -> ModeFlag<Toggle> {
    builder.mode_flag(key, default).declare().unwrap()
}

fn rules(catalog: &Catalog, key: &str) -> Vec<(InvariantRule, String, String)> {
    catalog
        .declaration(key)
        .unwrap()
        .invariants()
        .iter()
        .map(|invariant| {
            (
                invariant.rule(),
                invariant.variant().to_owned(),
                invariant.other().as_str().to_owned(),
            )
        })
        .collect()
}

/// The pair a draft-level setter could not state in both directions: the flag
/// declared first needs the handle of the flag declared second.
#[test]
fn a_pair_is_reciprocal_whichever_flag_was_declared_first() {
    let mut builder = vocabulary_builder();
    let ledger_v2 = flag(&mut builder, "ledger_v2", Toggle::Off);
    let ledger_audit = flag(&mut builder, "ledger_audit", Toggle::Off);

    builder
        .requires(&ledger_v2, Toggle::On, &ledger_audit, Toggle::On)
        .unwrap();
    builder
        .requires(&ledger_audit, Toggle::On, &ledger_v2, Toggle::On)
        .unwrap();

    let catalog = builder.build().unwrap();
    assert_eq!(
        rules(&catalog, "ledger_v2"),
        [(InvariantRule::Requires, "on".into(), "ledger_audit".into())]
    );
    assert_eq!(
        rules(&catalog, "ledger_audit"),
        [(InvariantRule::Requires, "on".into(), "ledger_v2".into())]
    );
}

#[test]
fn an_enum_flag_and_a_toggle_can_require_each_other() {
    let mut builder = vocabulary_builder();
    let style = builder
        .mode_flag("checkout_style", Checkout::Classic)
        .declare()
        .unwrap();
    let express_lane = flag(&mut builder, "express_lane", Toggle::Off);

    builder
        .requires(&style, Checkout::Express, &express_lane, Toggle::On)
        .unwrap();
    builder
        .requires(&express_lane, Toggle::On, &style, Checkout::Express)
        .unwrap();

    let catalog = builder.build().unwrap();
    let forward = &catalog.declaration("checkout_style").unwrap().invariants()[0];
    let backward = &catalog.declaration("express_lane").unwrap().invariants()[0];
    assert!(forward.holds("express", "on") && !forward.holds("express", "off"));
    assert!(backward.holds("on", "express") && !backward.holds("on", "classic"));
}

#[test]
fn the_order_of_declarations_and_of_statements_changes_no_digest() {
    let forward = {
        let mut builder = vocabulary_builder();
        let first = flag(&mut builder, "first", Toggle::Off);
        let second = flag(&mut builder, "second", Toggle::Off);
        builder
            .requires(&first, Toggle::On, &second, Toggle::On)
            .unwrap();
        builder
            .conflicts(&second, Toggle::On, &first, Toggle::Off)
            .unwrap();
        builder.both_or_none(&first, &second).unwrap();
        builder.build().unwrap()
    };
    let reversed = {
        let mut builder = vocabulary_builder();
        let second = flag(&mut builder, "second", Toggle::Off);
        let first = flag(&mut builder, "first", Toggle::Off);
        builder.both_or_none(&first, &second).unwrap();
        builder
            .conflicts(&second, Toggle::On, &first, Toggle::Off)
            .unwrap();
        builder
            .requires(&first, Toggle::On, &second, Toggle::On)
            .unwrap();
        builder.build().unwrap()
    };
    assert_eq!(forward.digest(), reversed.digest());
    assert_eq!(rules(&forward, "first"), rules(&reversed, "first"));
    assert_eq!(rules(&forward, "second"), rules(&reversed, "second"));
}

/// A reciprocal pair is checked in each direction against the code defaults.
#[test]
fn each_direction_must_hold_at_the_code_defaults() {
    let mut builder = vocabulary_builder();
    let quiet = flag(&mut builder, "quiet", Toggle::Off);
    let loud = flag(&mut builder, "loud", Toggle::On);

    // `quiet` is off, so "quiet on requires loud on" is not yet in play.
    builder
        .requires(&quiet, Toggle::On, &loud, Toggle::On)
        .unwrap();
    // `loud` is on and `quiet` is off, so "loud on requires quiet on" fails now.
    let refused = builder.requires(&loud, Toggle::On, &quiet, Toggle::On);
    assert!(matches!(
        refused,
        Err(BuildError::DefaultsViolateInvariant { key, other, .. })
            if key.as_str() == "loud" && other.as_str() == "quiet"
    ));
}

#[test]
fn a_symmetric_relationship_may_be_stated_from_both_sides() {
    let mut builder = vocabulary_builder();
    let left = flag(&mut builder, "left", Toggle::Off);
    let right = flag(&mut builder, "right", Toggle::Off);
    builder.both_or_none(&left, &right).unwrap();
    builder.both_or_none(&right, &left).unwrap();
    let catalog = builder.build().unwrap();
    assert_eq!(rules(&catalog, "left").len(), 1);
    assert_eq!(rules(&catalog, "right").len(), 1);
}

#[test]
fn a_refused_relationship_leaves_the_catalog_as_it_was() {
    let build = |attempt: bool| {
        let mut builder = vocabulary_builder();
        let first = flag(&mut builder, "first", Toggle::Off);
        let second = flag(&mut builder, "second", Toggle::On);
        if attempt {
            assert!(
                builder
                    .requires(&first, Toggle::Off, &second, Toggle::Off)
                    .is_err()
            );
            assert!(
                builder
                    .requires(&second, Toggle::On, &second, Toggle::On)
                    .is_err()
            );
        }
        builder.build().unwrap()
    };
    assert_eq!(build(true).digest(), build(false).digest());
    assert!(rules(&build(true), "first").is_empty());
}

#[test]
fn a_flag_of_another_catalog_is_refused_on_either_side() {
    let mut other = vocabulary_builder();
    let outsider = flag(&mut other, "first", Toggle::Off);
    let mut builder = vocabulary_builder();
    let insider = flag(&mut builder, "first", Toggle::Off);

    let as_other = builder.requires(&insider, Toggle::On, &outsider, Toggle::On);
    assert!(matches!(as_other, Err(BuildError::ForeignHandle { .. })));
    let as_subject = builder.conflicts(&outsider, Toggle::On, &insider, Toggle::On);
    assert!(matches!(as_subject, Err(BuildError::ForeignHandle { .. })));
    let toggles = builder.both_or_none(&outsider, &insider);
    assert!(matches!(toggles, Err(BuildError::ForeignHandle { .. })));
    assert_eq!(insider.key().as_str(), "first");
}
