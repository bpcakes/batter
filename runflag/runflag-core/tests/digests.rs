//! Golden digest vectors and what each digest does and does not cover.
//!
//! The pinned values were computed independently of this crate, from the
//! framing the `digest` module documents, so they check the specification
//! rather than echo the implementation.

#[path = "support/fixtures.rs"]
mod fixtures;

use fixtures::{Change, Checkout, CheckoutWide, date, reference, vocabulary_builder};
use runflag_core::{
    CatalogDigest, DeclarationFingerprint, Flag as _, SafeState, StaleRule, Toggle, ToggleLiterals,
};

const REFERENCE_CATALOG: &str = "521b7905919625ab87c0bad0cf6e61abe2b54c6ede5e93e52ef8d9957ae65255";
const EMPTY_CATALOG: &str = "4660ffe3fe354fcf603405ab46cb03c6ffa104c6f423bf39d3fe283227932878";
const NEW_CHECKOUT: &str = "fd837dee0537fff88c2184db298d51c6c3c1ee36a7824f029d05f72ed46863cc";
const CHECKOUT_STYLE: &str = "01cb5d85b06a6d1eef15b58cc57b50b9950e62cf5c92b21aee5eb4312d0774ca";
const EXPORT_HALT: &str = "6673004015e8b807fca5c846ddfb24c3ae497e13990a66f1d95a69c8c83f3235";

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
