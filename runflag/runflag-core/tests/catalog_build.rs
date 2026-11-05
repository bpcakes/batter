//! Catalog build rules: vocabularies, keys, setters, ownership and
//! governing switches.

#[path = "support/fixtures.rs"]
mod fixtures;

use fixtures::{builder, expiry, vocabulary_builder};
use runflag_core::{
    Absence, BuildError, CatalogBuilder, FlipDiscipline, KeyError, ModeDeclaration, Owners,
    ReleaseDeclaration, SafeState, StaleRule, Toggle,
};

fn halt(builder: &mut CatalogBuilder) -> runflag_core::KillSwitch {
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
        .owners(Owners::Only(std::slice::from_ref(&foreign_kind)))
        .declare();
    assert!(
        matches!(owners, Err(BuildError::ForeignProcessKind { kind, .. }) if &*kind == "serving")
    );
    let serving = builder.process_kind("reporting").unwrap();
    let pinned = builder
        .mode_flag("strict_mode", Toggle::Off)
        .owners(Owners::Only(&[serving]))
        .pinned_by(&[foreign_kind])
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
    let mut builder = vocabulary_builder();
    let result = builder
        .mode_flag("bad-key", Toggle::Off)
        .owners(Owners::Only(&[]))
        .requires_settings(Toggle::On, &["FLAG_UNDER_PREFIX"])
        .declare();
    assert!(matches!(result, Err(BuildError::InvalidKey { .. })));
    let first = builder
        .mode_flag("strict_mode", Toggle::Off)
        .owners(Owners::Only(&[]))
        .requires_settings(Toggle::On, &["FLAG_UNDER_PREFIX"])
        .declare();
    assert!(matches!(first, Err(BuildError::EmptyOwners { .. })));
}

#[test]
fn setters_may_be_called_once() {
    let mut builder = vocabulary_builder();
    let switch = halt(&mut builder);
    let twice = |result: Result<_, BuildError>, expected: &str| match result {
        Err(BuildError::RepeatedCall { method, .. }) => assert_eq!(method, expected),
        other => panic!("expected a repeated {expected}, got {other:?}"),
    };
    twice(
        builder
            .mode_flag("a", Toggle::Off)
            .owners(Owners::All)
            .owners(Owners::All)
            .declare()
            .map(drop),
        "owners",
    );
    twice(
        builder
            .mode_flag("a", Toggle::Off)
            .absence(Absence::Default)
            .absence(Absence::Required)
            .declare()
            .map(drop),
        "absence",
    );
    twice(
        builder
            .mode_flag("a", Toggle::Off)
            .flip_discipline(FlipDiscipline::Rolling)
            .flip_discipline(FlipDiscipline::Rolling)
            .declare()
            .map(drop),
        "flip_discipline",
    );
    twice(
        builder
            .release_flag("a", Toggle::Off, expiry())
            .governed_by(&switch)
            .governed_by(&switch)
            .declare()
            .map(drop),
        "governed_by",
    );
    let mut pins = fixtures::builder();
    let serving = pins.process_kind("serving").unwrap();
    let worker = pins.process_kind("worker").unwrap();
    twice(
        pins.mode_flag("a", Toggle::Off)
            .owners(Owners::Only(std::slice::from_ref(&serving)))
            .pinned_by(std::slice::from_ref(&worker))
            .pinned_by(std::slice::from_ref(&worker))
            .declare()
            .map(drop),
        "pinned_by",
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

#[test]
fn owners_and_pinned_by_are_validated() {
    let mut builder = builder();
    let serving = builder.process_kind("serving").unwrap();
    let worker = builder.process_kind("worker").unwrap();
    builder.environment_class("hosted").unwrap();
    assert!(matches!(
        strict_mode(&mut builder)
            .owners(Owners::Only(&[]))
            .declare(),
        Err(BuildError::EmptyOwners { .. })
    ));
    let duplicate = strict_mode(&mut builder)
        .owners(Owners::Only(&[serving.clone(), serving.clone()]))
        .declare();
    assert!(
        matches!(duplicate, Err(BuildError::DuplicateKind { kind, .. }) if &*kind == "serving")
    );
    let duplicate_pins = strict_mode(&mut builder)
        .owners(Owners::Only(std::slice::from_ref(&serving)))
        .pinned_by(&[worker.clone(), worker.clone()])
        .declare();
    assert!(
        matches!(duplicate_pins, Err(BuildError::DuplicateKind { kind, .. }) if &*kind == "worker")
    );
    let empty_pins = strict_mode(&mut builder)
        .owners(Owners::Only(std::slice::from_ref(&serving)))
        .pinned_by(&[])
        .declare();
    assert!(matches!(empty_pins, Err(BuildError::EmptyPinnedBy { .. })));
    let owner_pins = strict_mode(&mut builder)
        .owners(Owners::Only(std::slice::from_ref(&serving)))
        .pinned_by(std::slice::from_ref(&serving))
        .declare();
    assert!(
        matches!(owner_pins, Err(BuildError::PinnedByOwner { kind, .. }) if &*kind == "serving")
    );
    let all_pins = strict_mode(&mut builder)
        .pinned_by(std::slice::from_ref(&worker))
        .declare();
    assert!(matches!(
        all_pins,
        Err(BuildError::PinnedByWithAllOwners { .. })
    ));
    // `pinned_by` before `owners` is validated against the final owners.
    let reordered = strict_mode(&mut builder)
        .pinned_by(std::slice::from_ref(&worker))
        .owners(Owners::Only(std::slice::from_ref(&serving)));
    reordered.declare().unwrap();
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
        .owners(Owners::Only(std::slice::from_ref(&serving)))
        .declare()
        .unwrap();

    let owner_without = release(&mut builder, "a")
        .owners(Owners::Only(std::slice::from_ref(&worker)))
        .governed_by(&serving_switch)
        .declare();
    assert!(
        matches!(owner_without, Err(BuildError::GoverningSwitchNotOwned { kind, .. }) if &*kind == "worker")
    );
    let pinner_without = release(&mut builder, "a")
        .owners(Owners::Only(std::slice::from_ref(&serving)))
        .pinned_by(std::slice::from_ref(&worker))
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
        .owners(Owners::Only(std::slice::from_ref(&serving)))
        .governed_by(&serving_switch)
        .declare()
        .unwrap();
    let everywhere = halt(&mut builder);
    release(&mut builder, "b")
        .owners(Owners::Only(&[serving]))
        .pinned_by(&[worker])
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
