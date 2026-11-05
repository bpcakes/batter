//! Golden digest vectors and what each digest does and does not cover.
//!
//! The pinned values were computed independently of this crate, from the
//! framing the `digest` module documents, so they check the specification
//! rather than echo the implementation.

#[path = "support/fixtures.rs"]
mod fixtures;

use batter_flags_core::{
    CatalogDigest, DeclarationFingerprint, Flag as _, SafeState, StaleRule, Toggle, ToggleLiterals,
};
use fixtures::{Change, Checkout, CheckoutWide, date, reference, vocabulary_builder};

const REFERENCE_CATALOG: &str = "8b78ff2a3c0ce7572c2e8c2f6964674f4cb5951c9e87c60e1cd5973029fce8db";
const EMPTY_CATALOG: &str = "ccec4ecfaf692ae86d21dd01324e9604c07481641385da7830b7498382425c6a";
const NEW_CHECKOUT: &str = "f5ca7c0886c361c1912c690e531f4ae0ebbe9507d319f45a61bc44dc432f6ae9";
const CHECKOUT_STYLE: &str = "4d95da13ae9ae1dc5596496d003e37592a303391c62b84e818f373212facaad5";
const EXPORT_HALT: &str = "5fb458956635fe76022fde1448107348d7bcf32e94107e7a474739168d064233";
/// A catalog with one flag carrying a `requires`, a `conflicts` and a
/// `both_or_none` invariant, framed with invariants sorted by rule tag text.
const MIXED_INVARIANTS: &str = "592cc6263c63f842508864b4ce23fa1e6898adf6057eef2a1877b15b22926e0e";

fn digest(change: Change) -> CatalogDigest {
    reference(change).digest()
}

#[test]
fn the_reference_catalog_digest_is_pinned() {
    let catalog = reference(Change::None);
    assert_eq!(catalog.digest().to_hex(), REFERENCE_CATALOG);
    assert_eq!(
        REFERENCE_CATALOG.parse::<CatalogDigest>().unwrap(),
        catalog.digest()
    );
}

#[test]
fn an_empty_catalog_digest_is_pinned() {
    let mut builder = fixtures::builder();
    builder.process_kind("serving").unwrap();
    builder.environment_class("hosted").unwrap();
    assert_eq!(builder.build().unwrap().digest().to_hex(), EMPTY_CATALOG);
}

#[test]
fn declaration_fingerprints_are_pinned() {
    let catalog = reference(Change::None);
    let fingerprint = |key| catalog.declaration(key).unwrap().fingerprint().to_hex();
    assert_eq!(fingerprint("new_checkout"), NEW_CHECKOUT);
    assert_eq!(fingerprint("checkout_style"), CHECKOUT_STYLE);
    assert_eq!(fingerprint("export_halt"), EXPORT_HALT);
    let mut builder = vocabulary_builder();
    let handle = builder
        .release_flag("new_checkout", Toggle::On, fixtures::expiry())
        .declare()
        .unwrap();
    assert_eq!(
        handle.fingerprint().to_hex(),
        NEW_CHECKOUT,
        "defaults and policy are not fingerprinted"
    );
    let switch = builder
        .kill_switch("export_halt", SafeState::Released, StaleRule::HoldLastKnown)
        .declare()
        .unwrap();
    assert_eq!(switch.fingerprint().to_hex(), EXPORT_HALT);
}

#[test]
fn fingerprints_change_with_kind_value_type_and_variants() {
    let mut builder = vocabulary_builder();
    let release = builder
        .release_flag("a", Checkout::Classic, fixtures::expiry())
        .declare()
        .unwrap();
    let mut other = vocabulary_builder();
    let mode = other.mode_flag("a", Checkout::Classic).declare().unwrap();
    let mut wider = vocabulary_builder();
    let wide = wider
        .mode_flag("a", CheckoutWide::Classic)
        .declare()
        .unwrap();
    let mut toggles = vocabulary_builder();
    let toggle = toggles.mode_flag("a", Toggle::Off).declare().unwrap();
    let all: [DeclarationFingerprint; 4] = [
        release.fingerprint(),
        mode.fingerprint(),
        wide.fingerprint(),
        toggle.fingerprint(),
    ];
    for (index, left) in all.iter().enumerate() {
        for right in &all[index + 1..] {
            assert_ne!(left, right);
        }
    }
}

#[test]
fn declaration_order_changes_no_digest() {
    assert_eq!(digest(Change::ReversedOrder).to_hex(), REFERENCE_CATALOG);
}

#[test]
fn the_catalog_digest_covers_every_behaviour_bearing_field() {
    let base = digest(Change::None);
    let cases = [
        ("prefix", Change::Prefix("APP_FLAGS_")),
        (
            "toggle literals",
            Change::ToggleLiterals(ToggleLiterals::OnOff),
        ),
        ("process kinds", Change::ExtraKind),
        ("environment classes", Change::ExtraClass),
        ("retired names", Change::ExtraRetired),
        ("owners", Change::NewCheckoutOwnedByAll),
        ("pinned_by", Change::NewCheckoutUnpinned),
        ("only_in", Change::DebugPanelEverywhere),
        ("absence", Change::ExportHaltAbsenceDefault),
        (
            "safe state",
            Change::ExportHaltSafeState(SafeState::Released),
        ),
        (
            "stale rule",
            Change::ExportHaltStaleRule(StaleRule::HoldLastKnown),
        ),
        ("governing switch", Change::NewCheckoutUngoverned),
        ("invariants", Change::WithoutInvariant),
        ("required settings", Change::WithoutRequiresSettings),
    ];
    for (field, change) in cases {
        assert_ne!(digest(change), base, "the catalog digest ignores {field}");
    }
    assert_ne!(
        digest(Change::ExpressDefaultWithoutInvariant),
        digest(Change::WithoutInvariant),
        "the catalog digest ignores defaults"
    );
}

#[test]
fn expiry_flip_discipline_and_migration_notes_leave_the_catalog_digest() {
    let cases = [
        Change::NewCheckoutExpiry(date(2030, 1, 1)),
        Change::DebugPanelRolling,
        Change::MigrationNote("delete it"),
    ];
    for change in cases {
        assert_eq!(digest(change).to_hex(), REFERENCE_CATALOG, "{change:?}");
    }
}

#[cfg(feature = "serde")]
#[test]
fn the_manifest_digest_covers_what_the_catalog_digest_leaves_out() {
    let base = reference(Change::None).manifest_digest();
    let cases = [
        ("expiry", Change::NewCheckoutExpiry(date(2030, 1, 1))),
        ("flip discipline", Change::DebugPanelRolling),
        ("migration notes", Change::MigrationNote("delete it")),
        ("owners", Change::NewCheckoutOwnedByAll),
    ];
    for (field, change) in cases {
        assert_ne!(
            reference(change).manifest_digest(),
            base,
            "the manifest digest ignores {field}"
        );
    }
    assert_eq!(
        reference(Change::ReversedOrder).manifest_digest(),
        base,
        "declaration order changes the manifest digest"
    );
}

/// Builds the mixed-invariant catalog, declaring the three invariants in the
/// order `order` lists them.
fn mixed_invariants(order: [batter_flags_core::InvariantRule; 3]) -> batter_flags_core::Catalog {
    use batter_flags_core::InvariantRule;
    let mut builder = fixtures::builder();
    builder.process_kind("serving").unwrap();
    builder.environment_class("hosted").unwrap();
    let base = builder.mode_flag("base", Toggle::Off).declare().unwrap();
    let other = builder.mode_flag("other", Toggle::Off).declare().unwrap();
    let mut subject = builder.mode_flag("subject", Toggle::Off);
    for rule in order {
        subject = match rule {
            InvariantRule::Requires => subject.requires(Toggle::On, &base, Toggle::On),
            InvariantRule::Conflicts => subject.conflicts(Toggle::On, &other, Toggle::On),
            InvariantRule::BothOrNone => subject.both_or_none(&base),
        };
    }
    subject.declare().unwrap();
    builder.build().unwrap()
}

#[test]
fn invariants_of_different_rules_sort_by_rule_tag_text() {
    use batter_flags_core::InvariantRule::{BothOrNone, Conflicts, Requires};
    let catalog = mixed_invariants([Requires, Conflicts, BothOrNone]);
    assert_eq!(catalog.digest().to_hex(), MIXED_INVARIANTS);
    let reordered = mixed_invariants([BothOrNone, Requires, Conflicts]);
    assert_eq!(reordered.digest().to_hex(), MIXED_INVARIANTS);
    let rules: Vec<&str> = catalog
        .declaration("subject")
        .unwrap()
        .invariants()
        .iter()
        .map(|invariant| invariant.rule().as_str())
        .collect();
    assert_eq!(rules, ["both_or_none", "conflicts", "requires"]);
    assert!(BothOrNone < Conflicts && Conflicts < Requires);
}
