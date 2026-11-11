//! Revisions and the startup report: R-10 and R-11.
//!
//! The pinned values are the ones `batter-flags-core` pins for its own
//! resolution path, computed independently of the implementation from the
//! framing its `digest` module documents; resolving through declared mode must
//! land on the same digests, because the revision is a function of the
//! (key, resolved variant) pairs and nothing else.
#![allow(clippy::unwrap_used, reason = "a panic fails the test")]
#![allow(clippy::missing_assert_message, reason = "test names locate failures")]

#[path = "support/world.rs"]
mod world;

use batter_core::settings::SettingsSource;
use batter_flags::{
    ValueSource,
    core::{
        Catalog, CatalogBuilder, EnvironmentClass, FlagValue, ProcessKind, SafeState,
        SettingsPrefix, StaleRule, Toggle,
    },
    declared,
};
use world::{
    As, Checkout, expiry, resolve_as, resolve_development, resolve_serving, resolve_worker, today,
};

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

fn builder() -> (CatalogBuilder, ProcessKind, EnvironmentClass) {
    let (builder, serving, hosted) =
        CatalogBuilder::new(SettingsPrefix::new("FLAG_").unwrap(), "serving", "hosted").unwrap();
    (builder, serving, hosted)
}

fn resolve_empty(
    catalog: Catalog,
    kind: ProcessKind,
    class: EnvironmentClass,
) -> declared::Resolved {
    let source = SettingsSource::from_pairs([]).unwrap();
    declared::resolve(catalog, &source, kind, class, today()).unwrap()
}

#[test]
fn the_revision_frames_the_key_apart_from_the_variant() {
    let (mut ab, serving, hosted) = builder();
    ab.mode_flag("ab", Short).declare().unwrap();
    let ab = resolve_empty(ab.build(), serving, hosted);
    assert_eq!(ab.revision().to_hex(), KEY_AB_VARIANT_C);

    let (mut a, serving, hosted) = builder();
    a.mode_flag("a", Long).declare().unwrap();
    let a = resolve_empty(a.build(), serving, hosted);
    assert_eq!(a.revision().to_hex(), KEY_A_VARIANT_BC);

    // ("ab", "c") and ("a", "bc") concatenate alike and must not collide.
    assert_ne!(ab.revision(), a.revision());
}

#[test]
fn a_catalog_with_nothing_declared_has_the_empty_revision() {
    let (empty, serving, hosted) = builder();
    let resolved = resolve_empty(empty.build(), serving, hosted);
    assert_eq!(resolved.revision().to_hex(), EMPTY);
    assert_eq!(resolved.shared_revision().to_hex(), EMPTY);
}

/// The seven declarations of the `batter-flags-core` revision vectors, with
/// the owners that decide which of them are shared.
fn core_vectors_catalog() -> (Catalog, ProcessKind, ProcessKind, EnvironmentClass) {
    let (mut builder, serving, hosted) = builder();
    let worker = builder.process_kind("worker").unwrap();
    let development = builder.environment_class("development").unwrap();
    let checkout_halt = builder
        .kill_switch(
            "checkout_halt",
            SafeState::Released,
            StaleRule::HoldLastKnown,
        )
        .declare()
        .unwrap();
    builder
        .kill_switch(
            "export_halt",
            SafeState::Engaged,
            StaleRule::EngageWhenStale,
        )
        .owned_only_by(&worker)
        .declare()
        .unwrap();
    builder
        .release_flag("new_checkout", Toggle::Off, expiry())
        .owned_only_by(&serving)
        .pinned_by(&worker)
        .governed_by(&checkout_halt)
        .requires_settings(Toggle::On, &["PAYMENT_API_URL"])
        .declare()
        .unwrap();
    builder
        .release_flag("fast_path", Toggle::Off, expiry())
        .declare()
        .unwrap();
    builder
        .mode_flag("checkout_style", Checkout::Classic)
        .declare()
        .unwrap();
    builder
        .mode_flag("debug_panel", Toggle::Off)
        .only_in(&development)
        .declare()
        .unwrap();
    builder
        .mode_flag("worker_mode", Toggle::Off)
        .owned_only_by(&worker)
        .declare()
        .unwrap();
    (builder.build(), serving, worker, hosted)
}

/// (settings, revision over every declaration, revision over `Owners::All`).
type Vector = (
    &'static [(&'static str, &'static str)],
    &'static str,
    &'static str,
);

const SERVING: [Vector; 3] = [
    (
        &[],
        "4d1bea64c4703afaeee889422bb2d76811fc14a7f4ea9863a3b2538a1db46eb7",
        "ea6cc1d29f4262f932237077dcc181339813090d9eb27a00bc6f4b314d6807c5",
    ),
    (
        &[
            ("FLAG_NEW_CHECKOUT", "on"),
            ("FLAG_CHECKOUT_HALT", "engaged"),
            ("PAYMENT_API_URL", "https://payments.example"),
        ],
        "42594cdad7a53ae6092330c9137d11bbb63c18cd782bf61224d46dc5e69c56c9",
        "8681083412ddcb757c1ecd958c27018fdaf8ffaf95f3059e54f97061814330ba",
    ),
    (
        &[("FLAG_CHECKOUT_STYLE", "express")],
        "9d0c550290eef84a7b1abff855433fb3216ba33475f1466719a7456123475bd4",
        "27676d4d9b4b099cd82bbd040f43d48797c75f5efae71923a3ec556173e28484",
    ),
];

const WORKER: [Vector; 2] = [
    (
        &[("FLAG_WORKER_MODE", "on")],
        "d4d5c65019011822e3d19631fff7dd901375d394aaa1523255410de6488ca45d",
        "ea6cc1d29f4262f932237077dcc181339813090d9eb27a00bc6f4b314d6807c5",
    ),
    (
        &[("FLAG_FAST_PATH", "on")],
        "990bceb8495123f402bb08b65cef1187d513d73da6d0eee0a83fdaf2863ddf60",
        "55f132ef7873c6ba97935a813bb428872682ea8ee568c356b4ea012437770422",
    ),
];

fn vector_source(settings: &[(&str, &str)]) -> SettingsSource {
    SettingsSource::from_pairs(
        settings
            .iter()
            .map(|(name, value)| ((*name).into(), (*value).into())),
    )
    .unwrap()
}

#[test]
fn the_revisions_of_a_resolved_catalog_are_pinned() {
    for (settings, all, shared) in SERVING {
        let (catalog, serving, _worker, hosted) = core_vectors_catalog();
        let resolved =
            declared::resolve(catalog, &vector_source(settings), serving, hosted, today()).unwrap();
        assert_eq!(resolved.revision().to_hex(), all, "{settings:?}");
        assert_eq!(resolved.shared_revision().to_hex(), shared, "{settings:?}");
    }
    for (settings, all, shared) in WORKER {
        let (catalog, _serving, worker, hosted) = core_vectors_catalog();
        let resolved =
            declared::resolve(catalog, &vector_source(settings), worker, hosted, today()).unwrap();
        assert_eq!(resolved.revision().to_hex(), all, "{settings:?}");
        assert_eq!(resolved.shared_revision().to_hex(), shared, "{settings:?}");
    }
}

#[test]
fn the_revision_does_not_depend_on_the_order_the_source_was_assembled_in() {
    let settings = [
        ("FLAG_FAST_PATH", "on"),
        ("FLAG_CHECKOUT_STYLE", "express"),
        ("FLAG_LEDGER_AUDIT", "true"),
    ];
    let forward = resolve_serving(&settings).unwrap().0;
    let mut reversed = settings;
    reversed.reverse();
    let backward = resolve_serving(&reversed).unwrap().0;
    assert_eq!(forward.revision(), backward.revision());
    assert_eq!(forward.shared_revision(), backward.shared_revision());
}

#[test]
fn the_revision_is_the_resolved_set_not_its_spelling() {
    // `TRUE`, `on` and `On` are one variant: the same set, the same revision.
    let a = resolve_serving(&[("FLAG_FAST_PATH", "TRUE")]).unwrap().0;
    let b = resolve_serving(&[("FLAG_FAST_PATH", "on")]).unwrap().0;
    assert_eq!(a.revision(), b.revision());
    // An explicit default is the same set as an absent setting.
    let explicit = resolve_serving(&[("FLAG_FAST_PATH", "off")]).unwrap().0;
    let absent = resolve_serving(&[]).unwrap().0;
    assert_eq!(explicit.revision(), absent.revision());
    // A different resolved variant changes it, and the shared one too.
    assert_ne!(a.revision(), absent.revision());
    assert_ne!(a.shared_revision(), absent.shared_revision());
}

#[test]
fn the_shared_revision_ignores_what_only_one_kind_resolves() {
    let serving = resolve_as(
        As::ServingHosted,
        &[
            ("FLAG_NEW_CHECKOUT", "on"),
            ("PAYMENT_API_URL", "u"),
            ("PAYMENT_API_REGION", "r"),
        ],
    )
    .unwrap()
    .0;
    let worker = resolve_as(As::WorkerHosted, &[]).unwrap().0;
    assert_eq!(serving.shared_revision(), worker.shared_revision());
    assert_ne!(serving.revision(), worker.revision());
}

#[test]
fn the_report_lists_every_key_with_its_variant_and_source_and_both_revisions() {
    let (resolved, _) = resolve_serving(&[
        ("FLAG_FAST_PATH", "On"),
        ("FLAG_CHECKOUT_STYLE", "express"),
        ("FLAG_CHECKOUT_HALT", "engaged"),
    ])
    .unwrap();
    let report = resolved.report();
    assert_eq!(report.revision(), resolved.revision());
    assert_eq!(report.shared_revision(), resolved.shared_revision());
    let lines: Vec<String> = report.entries().iter().map(ToString::to_string).collect();
    let keys: Vec<&str> = report
        .entries()
        .iter()
        .map(|entry| entry.key().as_str())
        .collect();
    let mut sorted = keys.clone();
    sorted.sort_unstable();
    assert_eq!(keys, sorted, "key order");
    for expected in [
        "checkout_halt=engaged (literal)",
        "checkout_style=express (literal)",
        "fast_path=on (literal)",
        "bulk_export=off (absent)",
        "region_lock=off (literal)",
    ] {
        assert!(
            lines.iter().any(|line| line == expected),
            "{expected}: {lines:?}"
        );
    }
    // The worker-only flags appear for every kind, at their defaults where the
    // kind does not own them, and say that no value may be presented there.
    let worker_mode = report
        .entries()
        .iter()
        .find(|entry| entry.key().as_str() == "worker_mode")
        .unwrap();
    assert_eq!(
        (worker_mode.variant(), worker_mode.source()),
        ("off", ValueSource::NotApplicable)
    );
    assert!(
        lines
            .iter()
            .any(|line| line == "worker_mode=off (not_applicable)"),
        "{lines:?}"
    );
    let text = report.to_string();
    assert!(text.starts_with(&format!(
        "flags revision={} shared_revision={}\n",
        resolved.revision().to_hex(),
        resolved.shared_revision().to_hex()
    )));
}

/// How many entries of a report have each source.
fn tally(resolved: &declared::Resolved) -> (usize, usize, usize) {
    let report = resolved.report();
    let count = |wanted: ValueSource| {
        report
            .entries()
            .iter()
            .filter(|entry| entry.source() == wanted)
            .count()
    };
    (
        count(ValueSource::Literal),
        count(ValueSource::Absent),
        count(ValueSource::NotApplicable),
    )
}

#[test]
fn a_narrowed_source_shows_as_every_applicable_key_absent() {
    // The symptom the report exposes: a source that lost the flag names
    // resolved every flag it could have set to its default. The keys no value
    // may be presented for here do not hide it: they read `not_applicable`,
    // not `absent`.
    let (resolved, _) = resolve_serving(&[]).unwrap();
    // Seventeen declarations: eleven this kind may present a value for (one
    // carried by the required setting every world source has), four it does
    // not own and two restricted to the development class.
    assert_eq!(tally(&resolved), (1, 10, 6));
}

#[test]
fn what_may_be_presented_depends_on_the_kind_and_the_class() {
    // In `development`, the development-only flags are presentable too: the
    // required one came from a setting, the other is absent.
    let (resolved, _) = resolve_development(&[]).unwrap();
    assert_eq!(tally(&resolved), (2, 11, 4));
    // The worker owns its own flags and not the serving-only `new_checkout`.
    let (resolved, _) = resolve_worker(&[]).unwrap();
    let (literal, absent, not_applicable) = tally(&resolved);
    assert_eq!((literal, absent + not_applicable), (2, 15));
    let report = resolved.report();
    let source_of = |key: &str| {
        report
            .entries()
            .iter()
            .find(|entry| entry.key().as_str() == key)
            .unwrap()
            .source()
    };
    assert_eq!(source_of("worker_mode"), ValueSource::Absent);
    assert_eq!(source_of("worker_region"), ValueSource::Literal);
    assert_eq!(source_of("new_checkout"), ValueSource::NotApplicable);
    assert_eq!(source_of("debug_panel"), ValueSource::NotApplicable);
}
