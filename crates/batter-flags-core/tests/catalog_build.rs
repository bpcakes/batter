//! Catalog build rules: vocabularies, keys, setters, ownership and
//! governing switches.
#![allow(clippy::unwrap_used, reason = "a panic fails the test")]
#![allow(clippy::missing_assert_message, reason = "test names locate failures")]

#[path = "support/fixtures.rs"]
mod fixtures;

use batter_flags_core::{
    Absence, BuildError, CatalogBuilder, FlipDiscipline, KeyError, ModeDeclaration, Owners,
    ReleaseDeclaration, SafeState, StaleRule, Toggle,
};
use fixtures::{builder, expiry, vocabulary_builder};

fn halt(builder: &mut CatalogBuilder) -> batter_flags_core::KillSwitch {
    builder
        .kill_switch("halt", SafeState::Released, StaleRule::HoldLastKnown)
        .declare()
        .unwrap()
}

fn strict_mode(builder: &mut CatalogBuilder) -> ModeDeclaration<'_, Toggle> {
    builder.mode_flag("strict_mode", Toggle::Off)
}

fn release<'b>(builder: &'b mut CatalogBuilder, key: &str) -> ReleaseDeclaration<'b, Toggle> {
    builder.release_flag(key, Toggle::Off, expiry())
}

#[test]
fn vocabulary_names_follow_the_grammar_and_are_unique() {
    let mut builder = builder();
    assert!(matches!(
        builder.process_kind("Serving"),
        Err(BuildError::InvalidProcessKind {
            cause: KeyError::InvalidStart { character: 'S' },
            ..
        })
    ));
    assert!(matches!(
        builder.environment_class("hosted-eu"),
        Err(BuildError::InvalidEnvironmentClass {
            cause: KeyError::InvalidCharacter { .. },
            ..
        })
    ));
    let serving = builder.process_kind("serving").unwrap();
    assert_eq!(serving.name(), "serving");
    assert_eq!(
        builder.process_kind("serving"),
        Err(BuildError::DuplicateProcessKind {
            name: "serving".into()
        })
    );
    builder.environment_class("hosted").unwrap();
    assert_eq!(
        builder.environment_class("hosted"),
        Err(BuildError::DuplicateEnvironmentClass {
            name: "hosted".into()
        })
    );
    // The vocabularies are separate namespaces.
    builder.environment_class("serving").unwrap();
}

#[test]
fn a_catalog_needs_both_vocabularies() {
    assert_eq!(builder().build().unwrap_err(), BuildError::NoProcessKind);
    let mut only_kind = builder();
    only_kind.process_kind("serving").unwrap();
    assert_eq!(
        only_kind.build().unwrap_err(),
        BuildError::NoEnvironmentClass
    );
    let mut both = builder();
    both.process_kind("serving").unwrap();
    both.environment_class("hosted").unwrap();
    assert_eq!(both.build().unwrap().declarations().count(), 0);
}

#[test]
fn vocabulary_values_from_another_catalog_are_build_errors() {
    let mut other = builder();
    let foreign_kind = other.process_kind("serving").unwrap();
    let foreign_class = other.environment_class("development").unwrap();
    let mut builder = vocabulary_builder();
    let owners = builder
        .mode_flag("strict_mode", Toggle::Off)
        .owned_only_by(&foreign_kind)
        .declare();
    assert!(
        matches!(owners, Err(BuildError::ForeignProcessKind { kind, .. }) if &*kind == "serving")
    );
    let serving = builder.process_kind("reporting").unwrap();
    let pinned = builder
        .mode_flag("strict_mode", Toggle::Off)
        .owned_only_by(&serving)
        .pinned_by(&foreign_kind)
        .declare();
    assert!(matches!(pinned, Err(BuildError::ForeignProcessKind { .. })));
    let class = builder
        .mode_flag("strict_mode", Toggle::Off)
        .only_in(&foreign_class)
        .declare();
    assert!(
        matches!(class, Err(BuildError::ForeignEnvironmentClass { class, .. }) if &*class == "development")
    );
    // A same-named value from this catalog is accepted.
    builder.environment_class("staging").unwrap();
}

#[test]
fn keys_follow_the_grammar_and_are_unique_across_kinds() {
    let mut builder = vocabulary_builder();
    assert!(matches!(
        builder.mode_flag("Strict", Toggle::Off).declare(),
        Err(BuildError::InvalidKey {
            cause: KeyError::InvalidStart { character: 'S' },
            ..
        })
    ));
    builder
        .mode_flag("strict_mode", Toggle::Off)
        .declare()
        .unwrap();
    let duplicate = builder
        .kill_switch("strict_mode", SafeState::Released, StaleRule::HoldLastKnown)
        .declare();
    assert!(
        matches!(duplicate, Err(BuildError::DuplicateKey { key }) if key.as_str() == "strict_mode")
    );
}

#[test]
fn an_invalid_key_outranks_every_later_error() {
    let mut other = builder();
    let foreign_kind = other.process_kind("serving").unwrap();
    let mut builder = vocabulary_builder();
    let result = builder
        .mode_flag("bad-key", Toggle::Off)
        .owned_only_by(&foreign_kind)
        .requires_settings(Toggle::On, &["FLAG_UNDER_PREFIX"])
        .declare();
    assert!(matches!(result, Err(BuildError::InvalidKey { .. })));
    // Otherwise the first error is the one reported.
    let first = builder
        .mode_flag("strict_mode", Toggle::Off)
        .owned_only_by(&foreign_kind)
        .requires_settings(Toggle::On, &["FLAG_UNDER_PREFIX"])
        .declare();
    assert!(matches!(first, Err(BuildError::ForeignProcessKind { .. })));
}

/// A catalog with one flag whose every setter with a single possible meaning is
/// applied `times` times.
fn repeated(times: usize) -> batter_flags_core::Catalog {
    let mut builder = builder();
    let serving = builder.process_kind("serving").unwrap();
    let worker = builder.process_kind("worker").unwrap();
    builder.environment_class("hosted").unwrap();
    let mut flag = strict_mode(&mut builder).owned_only_by(&serving);
    for _ in 0..times {
        flag = flag
            .and_owned_by(&serving)
            .pinned_by(&worker)
            .required()
            .coordinated();
    }
    flag.declare().unwrap();
    builder.build().unwrap()
}

/// Repeating a setter that can mean only one thing is not an error: there is
/// nothing to choose between, so it changes nothing.
#[test]
fn setters_with_one_possible_meaning_are_idempotent() {
    let once = repeated(1);
    assert_eq!(once.digest(), repeated(3).digest());
    let declared = once.declaration("strict_mode").unwrap();
    assert_eq!(declared.absence(), Absence::Required);
    assert_eq!(declared.flip_discipline(), FlipDiscipline::Coordinated);
    assert_eq!(declared.pinned_by().len(), 1);
}

/// A second value would replace the first without a word, so it is refused.
#[test]
fn setters_that_would_replace_a_value_are_refused() {
    let mut builder = vocabulary_builder();
    let switch = halt(&mut builder);
    let twice = |result: Result<_, BuildError>, expected: &str| match result {
        Err(BuildError::RepeatedCall { method, .. }) => assert_eq!(method, expected),
        other => panic!("expected a repeated {expected}, got {other:?}"),
    };
    twice(
        builder
            .release_flag("a", Toggle::Off, expiry())
            .governed_by(&switch)
            .governed_by(&switch)
            .declare()
            .map(drop),
        "governed_by",
    );
    let mut classes = vocabulary_builder();
    let development = classes.environment_class("staging").unwrap();
    twice(
        classes
            .mode_flag("a", Toggle::Off)
            .only_in(&development)
            .only_in(&development)
            .declare()
            .map(drop),
        "only_in",
    );
}

/// Empty owner lists, pin readers without a restriction and a second
/// restriction do not compile (see the doctests on `owned_only_by` and
/// `pinned_by`). What is left to check is a relation between values.
#[test]
fn owners_and_pin_readers_are_validated() {
    let mut builder = builder();
    let serving = builder.process_kind("serving").unwrap();
    let worker = builder.process_kind("worker").unwrap();
    builder.environment_class("hosted").unwrap();

    // A kind cannot both own the flag and be a pin reader, whichever came first.
    let owner_pins = strict_mode(&mut builder)
        .owned_only_by(&serving)
        .pinned_by(&serving)
        .declare();
    assert!(
        matches!(owner_pins, Err(BuildError::PinnedByOwner { kind, .. }) if &*kind == "serving")
    );
    let reader_owns = strict_mode(&mut builder)
        .owned_only_by(&serving)
        .pinned_by(&worker)
        .and_owned_by(&worker)
        .declare();
    assert!(
        matches!(reader_owns, Err(BuildError::PinnedByOwner { kind, .. }) if &*kind == "worker")
    );

    // Naming a kind twice is not a conflict: the lists are sets.
    strict_mode(&mut builder)
        .owned_only_by(&serving)
        .and_owned_by(&serving)
        .pinned_by(&worker)
        .pinned_by(&worker)
        .declare()
        .unwrap();
    let catalog = builder.build().unwrap();
    let declared = catalog.declaration("strict_mode").unwrap();
    assert_eq!(
        declared.owners(),
        Owners::Only(std::slice::from_ref(&serving))
    );
    assert_eq!(declared.pinned_by(), std::slice::from_ref(&worker));
    assert_eq!(catalog.owned_by(&serving).unwrap().count(), 1);
    assert_eq!(
        catalog.owned_by(&worker).unwrap().count(),
        0,
        "pinned_by does not own"
    );
}

/// Owners are kept sorted by name, so the order they were added in changes no
/// digest.
#[test]
fn owners_are_canonical_whatever_order_they_were_added_in() {
    let build = |worker_first: &[&str]| {
        let mut builder = builder();
        let kinds: Vec<_> = ["serving", "worker"]
            .into_iter()
            .map(|name| builder.process_kind(name).unwrap())
            .collect();
        builder.environment_class("hosted").unwrap();
        let by_name = |name: &str| kinds.iter().find(|kind| kind.name() == name).unwrap();
        let mut flag = strict_mode(&mut builder).owned_only_by(by_name(worker_first[0]));
        flag = flag.and_owned_by(by_name(worker_first[1]));
        flag.declare().unwrap();
        builder.build().unwrap()
    };
    let (forward, reversed) = (build(&["serving", "worker"]), build(&["worker", "serving"]));
    assert_eq!(forward.digest(), reversed.digest());
    let owners = |catalog: &batter_flags_core::Catalog| match catalog
        .declaration("strict_mode")
        .unwrap()
        .owners()
    {
        Owners::Only(kinds) => kinds
            .iter()
            .map(|kind| kind.name().to_owned())
            .collect::<Vec<_>>(),
        Owners::All => Vec::new(),
    };
    assert_eq!(owners(&reversed), ["serving", "worker"]);
}

#[test]
fn a_governing_switch_must_belong_to_this_catalog() {
    let mut other = vocabulary_builder();
    let foreign = halt(&mut other);
    let mut builder = vocabulary_builder();
    halt(&mut builder);
    let result = builder
        .release_flag("new_checkout", Toggle::Off, expiry())
        .governed_by(&foreign)
        .declare();
    assert!(
        matches!(result, Err(BuildError::ForeignHandle { handle, .. }) if handle.as_str() == "halt")
    );
}

#[test]
fn every_kind_that_evaluates_or_resolves_a_flag_reads_its_switch() {
    let mut builder = builder();
    let serving = builder.process_kind("serving").unwrap();
    let worker = builder.process_kind("worker").unwrap();
    builder.environment_class("hosted").unwrap();
    let serving_switch = builder
        .kill_switch(
            "serving_halt",
            SafeState::Released,
            StaleRule::HoldLastKnown,
        )
        .owned_only_by(&serving)
        .declare()
        .unwrap();

    let owner_without = release(&mut builder, "a")
        .owned_only_by(&worker)
        .governed_by(&serving_switch)
        .declare();
    assert!(
        matches!(owner_without, Err(BuildError::GoverningSwitchNotOwned { kind, .. }) if &*kind == "worker")
    );
    let pinner_without = release(&mut builder, "a")
        .owned_only_by(&serving)
        .pinned_by(&worker)
        .governed_by(&serving_switch)
        .declare();
    assert!(
        matches!(pinner_without, Err(BuildError::GoverningSwitchNotOwned { kind, switch, .. })
        if &*kind == "worker" && switch.as_str() == "serving_halt")
    );
    let everyone = release(&mut builder, "a")
        .governed_by(&serving_switch)
        .declare();
    assert!(matches!(
        everyone,
        Err(BuildError::GoverningSwitchRestricted { .. })
    ));

    let governed = release(&mut builder, "a")
        .owned_only_by(&serving)
        .governed_by(&serving_switch)
        .declare()
        .unwrap();
    let everywhere = halt(&mut builder);
    release(&mut builder, "b")
        .owned_only_by(&serving)
        .pinned_by(&worker)
        .governed_by(&everywhere)
        .declare()
        .unwrap();
    let catalog = builder.build().unwrap();
    assert_eq!(
        catalog
            .declaration("a")
            .unwrap()
            .governed_by()
            .unwrap()
            .as_str(),
        "serving_halt"
    );
    let _ = governed;
}
