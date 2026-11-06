//! Revisions and snapshot digests: golden vectors, what each covers, and the
//! determinism they promise between processes.
//!
//! The pinned values were computed independently of this crate, from the
//! framing the `digest` module documents, so they check the specification
//! rather than echo the implementation.
#![allow(clippy::unwrap_used, reason = "a panic fails the test")]
#![allow(clippy::missing_assert_message, reason = "test names locate failures")]

#[path = "support/fixtures.rs"]
mod fixtures;
#[path = "support/world.rs"]
mod world;

use std::sync::Arc;

use batter_flags_core::{
    CatalogBuilder, Decision, FlagValue, Freshness, Revision, SettingsPrefix, SwitchState, Toggle,
    low_level::{Candidate, CandidateRow, Snapshot},
};
use fixtures::Checkout;
use world::{Literal, base, flag_row, snapshot, switch_row, undecodable_row, version, world};

const EMPTY: &str = "95f804b8ecdfc355842bf949b70925985beb1110bd0c3ef6de4a716181804412";
const KEY_AB_VARIANT_C: &str = "14b0cd9d1a4fe9b5c5c2c939b10453df552c641052573ee9842296ede0d92340";
const KEY_A_VARIANT_BC: &str = "2444d7ae1499ae85887b38315c64e75375f3ac859ca9509790e216c862079c20";

/// A one-variant value, so a catalog can declare the pair (`ab`, `c`).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct Short;

impl FlagValue for Short {
    const VARIANTS: &'static [&'static str] = &["c"];

    fn variant(self) -> &'static str {
        "c"
    }

    fn from_variant(variant: &str) -> Option<Self> {
        (variant == "c").then_some(Self)
    }
}

/// A one-variant value, so a catalog can declare the pair (`a`, `bc`).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct Long;

impl FlagValue for Long {
    const VARIANTS: &'static [&'static str] = &["bc"];

    fn variant(self) -> &'static str {
        "bc"
    }

    fn from_variant(variant: &str) -> Option<Self> {
        (variant == "bc").then_some(Self)
    }
}

/// The revision of a catalog that declares one mode flag.
fn single<T: FlagValue>(key: &str, default: T) -> Revision {
    let mut builder = CatalogBuilder::new(SettingsPrefix::new("FLAG_").unwrap());
    let serving = builder.process_kind("serving").unwrap();
    builder.environment_class("hosted").unwrap();
    builder.mode_flag(key, default).declare().unwrap();
    let catalog = builder.build().unwrap();
    base(catalog, &serving, &[]).revision()
}

#[test]
fn revisions_frame_the_key_apart_from_the_variant() {
    assert_eq!(single("ab", Short).to_hex(), KEY_AB_VARIANT_C);
    assert_eq!(single("a", Long).to_hex(), KEY_A_VARIANT_BC);
    assert_ne!(KEY_AB_VARIANT_C, KEY_A_VARIANT_BC);

    let mut builder = CatalogBuilder::new(SettingsPrefix::new("FLAG_").unwrap());
    let serving = builder.process_kind("serving").unwrap();
    builder.environment_class("hosted").unwrap();
    let empty = base(builder.build().unwrap(), &serving, &[]);
    assert_eq!(empty.revision().to_hex(), EMPTY);
    assert_eq!(empty.shared_revision().to_hex(), EMPTY);
}

/// (literals, revision over every declaration, revision over `Owners::All`).
type Vector = (&'static [Literal<'static>], &'static str, &'static str);

const SERVING: [Vector; 3] = [
    (
        &[],
        "4d1bea64c4703afaeee889422bb2d76811fc14a7f4ea9863a3b2538a1db46eb7",
        "ea6cc1d29f4262f932237077dcc181339813090d9eb27a00bc6f4b314d6807c5",
    ),
    (
        &[
            Literal::Flag("new_checkout", "on"),
            Literal::Switch("checkout_halt", SwitchState::Engaged),
        ],
        "42594cdad7a53ae6092330c9137d11bbb63c18cd782bf61224d46dc5e69c56c9",
        "8681083412ddcb757c1ecd958c27018fdaf8ffaf95f3059e54f97061814330ba",
    ),
    (
        &[Literal::Flag("checkout_style", "express")],
        "9d0c550290eef84a7b1abff855433fb3216ba33475f1466719a7456123475bd4",
        "27676d4d9b4b099cd82bbd040f43d48797c75f5efae71923a3ec556173e28484",
    ),
];

const WORKER: [Vector; 2] = [
    (
        &[Literal::Flag("worker_mode", "on")],
        "d4d5c65019011822e3d19631fff7dd901375d394aaa1523255410de6488ca45d",
        "ea6cc1d29f4262f932237077dcc181339813090d9eb27a00bc6f4b314d6807c5",
    ),
    (
        &[Literal::Flag("fast_path", "on")],
        "990bceb8495123f402bb08b65cef1187d513d73da6d0eee0a83fdaf2863ddf60",
        "55f132ef7873c6ba97935a813bb428872682ea8ee568c356b4ea012437770422",
    ),
];

#[test]
fn the_revisions_of_the_world_are_pinned() {
    for (literals, all, shared) in SERVING {
        let (catalog, h) = world();
        let resolved = base(catalog, &h.serving, literals);
        assert_eq!(resolved.revision().to_hex(), all, "{literals:?}");
        assert_eq!(resolved.shared_revision().to_hex(), shared, "{literals:?}");
    }
    for (literals, all, shared) in WORKER {
        let (catalog, h) = world();
        let resolved = base(catalog, &h.worker, literals);
        assert_eq!(resolved.revision().to_hex(), all, "{literals:?}");
        assert_eq!(resolved.shared_revision().to_hex(), shared, "{literals:?}");
    }
}

#[test]
fn the_shared_revision_ignores_what_only_one_kind_resolves() {
    let (catalog, h) = world();
    let serving = base(catalog, &h.serving, &[Literal::Flag("new_checkout", "on")]);
    let (catalog, h) = world();
    let worker = base(catalog, &h.worker, &[]);
    assert_eq!(
        serving.shared_revision(),
        worker.shared_revision(),
        "the one set every kind resolves"
    );
    assert_ne!(serving.revision(), worker.revision());

    // Two processes of one kind that resolved alike agree on both.
    let (catalog, h) = world();
    let again = base(catalog, &h.serving, &[Literal::Flag("new_checkout", "on")]);
    assert_eq!(serving.revision(), again.revision());
    assert_eq!(serving.shared_revision(), again.shared_revision());
}

const ROWS_V7: &str = "fba5ea5c3229da47cb1a4edf7b4d0c2ab81b52ad8a514a8bf4a2c17a667224e7";
const ROWS_V8: &str = "e72eb86088e48f4856a218ce3e967556f4babe7aeed77dd3a861f3f6d6c12b78";
const ROWS_JSONB_TEXT_V7: &str = "c13e3ea50a39ab2aefefbaff61b141bda48a2f32a4b1211702500fa55d8aa702";
const EMPTY_V0: &str = "82ab5c2752189a41d7c4b83c7982cd1c523050422833475cc92df85103c8ce81";
const EMPTY_V7: &str = "6d38cd64de85159df1e1525176cf33edaf104b348612cdcf5899baad474ad6e5";
const BOUNDARY_A_BC: &str = "4b305d265ba375c4e011e3d369676306094f65308f7f5bb494218cbd2e5e9e84";
const BOUNDARY_AB_C: &str = "909d523b7dee3a654ffdeee13963ad478786b5a5cc64bcff45c69f32e2d34f3c";

/// A candidate with a published flag, a published switch, an unknown key and a
/// held row, in an order no digest may depend on.
fn rows(new_checkout_text: &str) -> Vec<CandidateRow> {
    let row = CandidateRow::new;
    vec![
        row(
            "new_checkout",
            "release",
            "toggle",
            version(7),
            new_checkout_text,
        ),
        row(
            "checkout_halt",
            "switch",
            "switch",
            version(5),
            r#"{"encoding":1,"switch":"engaged"}"#,
        ),
        row(
            "retired_flag",
            "mode",
            "toggle",
            version(3),
            r#"{"encoding":1,"variant":"on"}"#,
        ),
        row(
            "checkout_style",
            "mode",
            "enum",
            version(6),
            r#"{"encoding":2,"variant":"express"}"#,
        ),
    ]
}

const ON: &str = r#"{"encoding":1,"variant":"on"}"#;

fn digest_of(at: u64, rows: Vec<CandidateRow>) -> String {
    let (catalog, h) = world();
    let base = base(catalog, &h.serving, &[]);
    snapshot(&base, at, rows).digest().to_hex()
}

#[test]
fn snapshot_digests_are_pinned() {
    assert_eq!(digest_of(0, vec![]), EMPTY_V0);
    assert_eq!(digest_of(7, vec![]), EMPTY_V7);
    assert_eq!(digest_of(7, rows(ON)), ROWS_V7);
    assert_eq!(digest_of(8, rows(ON)), ROWS_V8);
    // The jsonb spelling of the same state is different text, so a different digest.
    assert_eq!(
        digest_of(7, rows(r#"{"variant": "on", "encoding": 1}"#)),
        ROWS_JSONB_TEXT_V7
    );
    let boundary = |key: &str, kind: &str| {
        digest_of(1, vec![CandidateRow::new(key, kind, "x", version(1), "s")])
    };
    assert_eq!(boundary("a", "bc"), BOUNDARY_A_BC);
    assert_eq!(boundary("ab", "c"), BOUNDARY_AB_C);
}

#[test]
fn a_snapshot_digest_covers_the_candidate_as_read() {
    let reference = digest_of(7, rows(ON));
    let mut reversed = rows(ON);
    reversed.reverse();
    assert_eq!(digest_of(7, reversed), reference, "rows sort by key");

    // Held rows and unknown keys are part of what was read.
    let without = |index: usize| {
        let mut rows = rows(ON);
        rows.remove(index);
        rows
    };
    assert_ne!(digest_of(7, without(3)), reference, "the held row");
    assert_ne!(digest_of(7, without(2)), reference, "the unknown key");

    // Version, `updated` and every text change it.
    assert_ne!(digest_of(8, rows(ON)), reference);
    let mut moved = rows(ON);
    moved[0] = CandidateRow::new("new_checkout", "release", "toggle", version(8), ON);
    assert_ne!(digest_of(7, moved), reference);
    let mut retyped = rows(ON);
    retyped[0] = CandidateRow::new("new_checkout", "mode", "toggle", version(7), ON);
    assert_ne!(digest_of(7, retyped), reference);
}

#[test]
fn a_snapshot_digest_belongs_to_the_candidate_not_the_process() {
    let (first_catalog, first) = world();
    let first_base = base(first_catalog, &first.serving, &[]);
    let (second_catalog, second) = world();
    let second_base = base(
        second_catalog,
        &second.worker,
        &[Literal::Flag("worker_mode", "on")],
    );
    let read = |base: &batter_flags_core::low_level::Base| snapshot(base, 7, rows(ON)).digest();
    assert_eq!(read(&first_base), read(&second_base));
    assert_eq!(read(&first_base).to_hex(), ROWS_V7);
}

/// Everything a view reports about a decision except the handle behind it.
fn shape<T: Copy>(decision: &Decision<T>) -> (T, &'static str, String) {
    (
        decision.value(),
        decision.variant(),
        format!(
            "{:?} {:?} {:?}",
            decision.reason(),
            decision.layer(),
            decision.source()
        ),
    )
}

#[test]
fn two_processes_at_one_version_serve_identical_shared_values_whatever_their_start_order() {
    let state = |at: u64, fast: Toggle, style: Checkout, halt: SwitchState| {
        move |h: &world::Handles| {
            [
                flag_row(&h.fast_path, fast, at - 1),
                flag_row(&h.checkout_style, style, at - 1),
                switch_row(&h.checkout_halt, halt, at - 1),
            ]
        }
    };
    let early = state(3, Toggle::On, Checkout::Express, SwitchState::Engaged);
    let latest = state(9, Toggle::Off, Checkout::Classic, SwitchState::Released);

    // The serving process booted first, saw version 3 and then version 9.
    let (catalog, a) = world();
    let a_base = base(catalog, &a.serving, &[]);
    let scope = a_base.validation_scope();
    let a_boot = a_base.not_loaded_view(
        batter_flags_core::PrePublication::ServeDefaults,
        Freshness::Fresh,
    );
    assert_eq!(a_boot.evaluate(&a.fast_path).value(), Toggle::Off);
    let a_early = snapshot(&a_base, 3, early(&a));
    let a_late = Arc::new(
        Snapshot::validate(
            &scope,
            Candidate::new(&scope)
                .at(version(9))
                .rows(latest(&a))
                .carrying(&a_early)
                .unwrap(),
        )
        .unwrap(),
    );

    // The worker booted afterwards and read version 9 only.
    let (catalog, b) = world();
    let b_base = base(catalog, &b.worker, &[]);
    let b_late = snapshot(&b_base, 9, latest(&b));

    assert_eq!(a_late.digest(), b_late.digest());
    let a_view = a_base
        .control_plane_view(&a_late, Freshness::Fresh)
        .unwrap();
    let b_view = b_base
        .control_plane_view(&b_late, Freshness::Fresh)
        .unwrap();
    assert_eq!(
        shape(&a_view.evaluate(&a.fast_path)),
        shape(&b_view.evaluate(&b.fast_path))
    );
    assert_eq!(
        shape(&a_view.evaluate(&a.checkout_style)),
        shape(&b_view.evaluate(&b.checkout_style))
    );
    assert_eq!(
        shape(&a_view.switch(&a.checkout_halt)),
        shape(&b_view.switch(&b.checkout_halt))
    );
}

/// The held kill switch is the one history-dependent exception: releasing a
/// held engaged switch is unsafe, so a process that saw the engaged state keeps
/// it while a process that never saw it serves its safe state.
#[test]
fn the_held_kill_switch_is_the_one_history_dependent_exception() {
    let held = |h: &world::Handles, at: u64| {
        [
            flag_row(&h.fast_path, Toggle::On, at - 1),
            undecodable_row("checkout_halt", "switch", "switch", at - 1),
        ]
    };
    let (catalog, a) = world();
    let a_base = base(catalog, &a.serving, &[]);
    let scope = a_base.validation_scope();
    let saw_engaged = snapshot(
        &a_base,
        3,
        [switch_row(&a.checkout_halt, SwitchState::Engaged, 2)],
    );
    let a_late = Arc::new(
        Snapshot::validate(
            &scope,
            Candidate::new(&scope)
                .at(version(9))
                .rows(held(&a, 9))
                .carrying(&saw_engaged)
                .unwrap(),
        )
        .unwrap(),
    );
    let (catalog, b) = world();
    let b_base = base(catalog, &b.serving, &[]);
    let b_late = snapshot(&b_base, 9, held(&b, 9));

    assert_eq!(a_late.digest(), b_late.digest(), "they read the same rows");
    let a_view = a_base
        .control_plane_view(&a_late, Freshness::Fresh)
        .unwrap();
    let b_view = b_base
        .control_plane_view(&b_late, Freshness::Fresh)
        .unwrap();
    assert_eq!(
        shape(&a_view.evaluate(&a.fast_path)),
        shape(&b_view.evaluate(&b.fast_path)),
        "every release and mode flag is history-free"
    );
    assert_eq!(
        a_view.switch(&a.checkout_halt).value(),
        SwitchState::Engaged
    );
    assert_eq!(
        b_view.switch(&b.checkout_halt).value(),
        SwitchState::Released
    );
}
