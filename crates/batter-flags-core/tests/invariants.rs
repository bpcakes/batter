//! Control-plane invariants: a published entry that would leave a
//! `requires`, `conflicts` or `both_or_none` invariant unmet is held and
//! serves the declared base, so a snapshot never serves a pair declared mode
//! refuses to start with.
#![allow(clippy::unwrap_used, reason = "a panic fails the test")]
#![allow(clippy::missing_assert_message, reason = "test names locate failures")]

#[path = "support/fixtures.rs"]
mod fixtures;
#[path = "support/world.rs"]
mod world;

use std::sync::Arc;

use batter_flags_core::{
    Catalog, CatalogBuilder, Freshness, Layer, ModeFlag, ProcessKind, Reason, SettingsPrefix,
    Toggle,
    low_level::{Base, HoldCause, Snapshot},
};
use world::{Literal, base, flag_row, snapshot};

/// The flags of [`relations`].
///
/// | declaration | relation |
/// | --- | --- |
/// | `ledger_v2` | `on` requires `ledger_audit` `on`, whose `on` needs `AUDIT_SINK_URL` |
/// | `stage_three` | `on` requires `ledger_v2` `on` |
/// | `fast_lane` | `on` conflicts with `slow_lane` `on` |
/// | `pair_a` | both or none with `pair_b` |
/// | `serving_side` | owned by `serving`; `on` requires the `worker`-only `worker_side` `on` |
struct Flags {
    serving: ProcessKind,
    ledger_v2: ModeFlag<Toggle>,
    ledger_audit: ModeFlag<Toggle>,
    stage_three: ModeFlag<Toggle>,
    fast_lane: ModeFlag<Toggle>,
    slow_lane: ModeFlag<Toggle>,
    pair_a: ModeFlag<Toggle>,
    pair_b: ModeFlag<Toggle>,
    serving_side: ModeFlag<Toggle>,
}

fn relations() -> (Catalog, Flags) {
    let mut builder = CatalogBuilder::new(SettingsPrefix::new("FLAG_").unwrap());
    let serving = builder.process_kind("serving").unwrap();
    let worker = builder.process_kind("worker").unwrap();
    builder.environment_class("hosted").unwrap();
    let toggle = |builder: &mut CatalogBuilder, key: &str| {
        builder.mode_flag(key, Toggle::Off).declare().unwrap()
    };
    let ledger_v2 = toggle(&mut builder, "ledger_v2");
    let ledger_audit = builder
        .mode_flag("ledger_audit", Toggle::Off)
        .requires_settings(Toggle::On, &["AUDIT_SINK_URL"])
        .declare()
        .unwrap();
    let stage_three = toggle(&mut builder, "stage_three");
    let fast_lane = toggle(&mut builder, "fast_lane");
    let slow_lane = toggle(&mut builder, "slow_lane");
    let pair_a = toggle(&mut builder, "pair_a");
    let pair_b = toggle(&mut builder, "pair_b");
    let serving_side = builder
        .mode_flag("serving_side", Toggle::Off)
        .owned_only_by(&serving)
        .declare()
        .unwrap();
    let worker_side = builder
        .mode_flag("worker_side", Toggle::Off)
        .owned_only_by(&worker)
        .declare()
        .unwrap();
    builder
        .requires(&ledger_v2, Toggle::On, &ledger_audit, Toggle::On)
        .unwrap();
    builder
        .requires(&stage_three, Toggle::On, &ledger_v2, Toggle::On)
        .unwrap();
    builder
        .conflicts(&fast_lane, Toggle::On, &slow_lane, Toggle::On)
        .unwrap();
    builder.both_or_none(&pair_a, &pair_b).unwrap();
    builder
        .requires(&serving_side, Toggle::On, &worker_side, Toggle::On)
        .unwrap();
    let flags = Flags {
        serving,
        ledger_v2,
        ledger_audit,
        stage_three,
        fast_lane,
        slow_lane,
        pair_a,
        pair_b,
        serving_side,
    };
    (builder.build().unwrap(), flags)
}

fn held(snapshot: &Snapshot) -> Vec<(String, HoldCause)> {
    snapshot
        .held(batter_flags_core::Freshness::Fresh)
        .map(|(key, cause)| (key.as_str().to_owned(), cause))
        .collect()
}

fn unmet(keys: &[&str]) -> Vec<(String, HoldCause)> {
    keys.iter()
        .map(|key| ((*key).to_owned(), HoldCause::InvariantUnmet))
        .collect()
}

/// What `flag` serves from `snapshot`, with its reason and layer.
fn served(
    base: &Base,
    snapshot: &Arc<Snapshot>,
    flag: &ModeFlag<Toggle>,
) -> (Toggle, Reason, Layer) {
    let view = base.control_plane_view(snapshot, Freshness::Fresh).unwrap();
    let decision = view.evaluate(flag);
    (
        decision.value(),
        decision.reason(),
        decision.layer().clone(),
    )
}

#[test]
fn a_dependent_published_without_what_it_requires_is_held_at_its_base() {
    let (catalog, f) = relations();
    let base = base(catalog, &f.serving, &[]);
    let alone = snapshot(&base, 3, [flag_row(&f.ledger_v2, Toggle::On, 2)]);
    assert_eq!(held(&alone), unmet(&["ledger_v2"]));
    assert_eq!(
        served(&base, &alone, &f.ledger_v2),
        (Toggle::Off, Reason::Default, Layer::Base),
        "the held dependent serves its declared base"
    );
    // Once what it requires is published too, both are served.
    let both = snapshot(
        &base,
        4,
        [
            flag_row(&f.ledger_v2, Toggle::On, 2),
            flag_row(&f.ledger_audit, Toggle::On, 4),
        ],
    );
    assert_eq!(held(&both), []);
    assert_eq!(
        served(&base, &both, &f.ledger_v2),
        (Toggle::On, Reason::Static, Layer::Entry)
    );
    // Taking the requirement away again holds the dependent, not the requirement.
    let withdrawn = snapshot(
        &base,
        5,
        [
            flag_row(&f.ledger_v2, Toggle::On, 2),
            flag_row(&f.ledger_audit, Toggle::Off, 5),
        ],
    );
    assert_eq!(held(&withdrawn), unmet(&["ledger_v2"]));
    assert_eq!(
        served(&base, &withdrawn, &f.ledger_audit),
        (Toggle::Off, Reason::Static, Layer::Entry)
    );
}

#[test]
fn holding_one_flag_holds_whatever_required_it_in_turn() {
    let (catalog, f) = relations();
    let base = base(catalog, &f.serving, &[]);
    let chain = snapshot(
        &base,
        3,
        [
            flag_row(&f.stage_three, Toggle::On, 2),
            flag_row(&f.ledger_v2, Toggle::On, 2),
        ],
    );
    // `ledger_v2` lacks `ledger_audit`, and `stage_three` then lacks `ledger_v2`.
    assert_eq!(held(&chain), unmet(&["ledger_v2", "stage_three"]));
    assert_eq!(
        served(&base, &chain, &f.stage_three),
        (Toggle::Off, Reason::Default, Layer::Base)
    );
}

#[test]
fn a_requirement_turned_off_under_a_dependent_on_in_the_base_is_held_instead() {
    let (catalog, f) = relations();
    // The settings turned both on; the control plane then turns the requirement off.
    let base = base(
        catalog,
        &f.serving,
        &[
            Literal::Flag("ledger_v2", "on"),
            Literal::Flag("ledger_audit", "on"),
        ],
    );
    let off = snapshot(&base, 3, [flag_row(&f.ledger_audit, Toggle::Off, 2)]);
    assert_eq!(held(&off), unmet(&["ledger_audit"]));
    assert_eq!(
        served(&base, &off, &f.ledger_audit),
        (Toggle::On, Reason::Default, Layer::Base)
    );
    assert_eq!(
        served(&base, &off, &f.ledger_v2),
        (Toggle::On, Reason::Default, Layer::Base),
        "the dependent, never published, keeps serving its base"
    );
}

#[test]
fn of_two_conflicting_flags_the_one_the_conflict_is_declared_on_is_held() {
    let (catalog, f) = relations();
    let base = base(catalog, &f.serving, &[]);
    let both = snapshot(
        &base,
        3,
        [
            flag_row(&f.fast_lane, Toggle::On, 2),
            flag_row(&f.slow_lane, Toggle::On, 3),
        ],
    );
    assert_eq!(held(&both), unmet(&["fast_lane"]));
    assert_eq!(
        served(&base, &both, &f.fast_lane),
        (Toggle::Off, Reason::Default, Layer::Base)
    );
    assert_eq!(
        served(&base, &both, &f.slow_lane),
        (Toggle::On, Reason::Static, Layer::Entry)
    );
    let one = snapshot(&base, 4, [flag_row(&f.slow_lane, Toggle::On, 3)]);
    assert_eq!(held(&one), []);
}

#[test]
fn a_both_or_none_pair_is_served_only_together() {
    let (catalog, f) = relations();
    let base = base(catalog, &f.serving, &[]);
    for (rows, expected) in [
        (vec![flag_row(&f.pair_a, Toggle::On, 2)], unmet(&["pair_a"])),
        (vec![flag_row(&f.pair_b, Toggle::On, 2)], unmet(&["pair_b"])),
        (
            vec![
                flag_row(&f.pair_a, Toggle::On, 2),
                flag_row(&f.pair_b, Toggle::On, 3),
            ],
            vec![],
        ),
        (
            vec![
                flag_row(&f.pair_a, Toggle::On, 2),
                flag_row(&f.pair_b, Toggle::Off, 3),
            ],
            unmet(&["pair_a"]),
        ),
    ] {
        let read = snapshot(&base, 4, rows);
        assert_eq!(held(&read), expected);
        let (a, b) = (
            served(&base, &read, &f.pair_a).0,
            served(&base, &read, &f.pair_b).0,
        );
        assert_eq!(a, b, "the pair is served on together or not at all");
    }
}

#[test]
fn an_invariant_on_a_flag_another_kind_owns_is_not_judged() {
    let (catalog, f) = relations();
    let base = base(catalog, &f.serving, &[]);
    // `worker_side` is the worker's: this process cannot know what it serves.
    let read = snapshot(&base, 3, [flag_row(&f.serving_side, Toggle::On, 2)]);
    assert_eq!(held(&read), []);
    assert_eq!(
        served(&base, &read, &f.serving_side),
        (Toggle::On, Reason::Static, Layer::Entry)
    );
}

#[test]
fn a_held_entry_keeps_its_first_cause_before_any_invariant_is_judged() {
    let (catalog, f) = relations();
    let base = base(
        catalog,
        &f.serving,
        &[Literal::SettingsAbsent("ledger_audit", "on")],
    );
    // `ledger_audit` on is held for its settings, so `ledger_v2` lacks it.
    let read = snapshot(
        &base,
        3,
        [
            flag_row(&f.ledger_v2, Toggle::On, 2),
            flag_row(&f.ledger_audit, Toggle::On, 2),
        ],
    );
    assert_eq!(
        held(&read),
        [
            ("ledger_v2".to_owned(), HoldCause::InvariantUnmet),
            ("ledger_audit".to_owned(), HoldCause::RequiresSettingsAbsent),
        ]
    );
}

/// `a` on conflicts with `b` on, and `b` on with `c` on, declared in `order`.
fn chain_in(order: [&str; 3]) -> (Catalog, ProcessKind, Vec<ModeFlag<Toggle>>) {
    let mut builder = CatalogBuilder::new(SettingsPrefix::new("FLAG_").unwrap());
    let serving = builder.process_kind("serving").unwrap();
    builder.environment_class("hosted").unwrap();
    let mut flags: Vec<(String, ModeFlag<Toggle>)> = order
        .iter()
        .map(|key| {
            let flag = builder.mode_flag(key, Toggle::Off).declare().unwrap();
            ((*key).to_owned(), flag)
        })
        .collect();
    flags.sort_by(|a, b| a.0.cmp(&b.0));
    let flags: Vec<_> = flags.into_iter().map(|(_, flag)| flag).collect();
    // State the relationships in the declaration order too.
    let pairs = [(0, 1), (1, 2)];
    for &(left, right) in if order[0] == "a" {
        &pairs[..]
    } else {
        &[(1, 2), (0, 1)][..]
    } {
        builder
            .conflicts(&flags[left], Toggle::On, &flags[right], Toggle::On)
            .unwrap();
    }
    (builder.build().unwrap(), serving, flags)
}

#[test]
fn which_flag_is_held_never_depends_on_declaration_order() {
    let mut outcomes = Vec::new();
    for order in [["a", "b", "c"], ["c", "b", "a"], ["b", "c", "a"]] {
        let (catalog, serving, flags) = chain_in(order);
        let digest = catalog.digest();
        let base = base(catalog, &serving, &[]);
        let read = snapshot(
            &base,
            3,
            flags.iter().map(|flag| flag_row(flag, Toggle::On, 2)),
        );
        let mut holds = held(&read);
        holds.sort_by(|a, b| a.0.cmp(&b.0));
        outcomes.push((digest, holds));
    }
    assert!(
        outcomes.windows(2).all(|pair| pair[0] == pair[1]),
        "one catalog, one outcome: {outcomes:?}"
    );
    assert_eq!(outcomes[0].1, unmet(&["a", "b"]));
}

#[cfg(feature = "serde")]
#[test]
fn a_judgement_that_no_hold_can_meet_is_reported_unresolved() {
    use batter_flags_core::FlagStanding;
    let (catalog, _) = relations();
    let manifest = catalog.manifest();
    // Both stand fixed on, so `fast_lane`'s conflict cannot be met.
    let judgement = manifest.invariant_holds("serving", |entry| {
        Some(match entry.key().as_str() {
            "fast_lane" | "slow_lane" => FlagStanding::Fixed("on"),
            _ => FlagStanding::Fixed("off"),
        })
    });
    assert!(judgement.holds.is_empty());
    assert_eq!(judgement.unresolved.len(), 1);
    assert_eq!(judgement.unresolved[0].0.as_str(), "fast_lane");
}
