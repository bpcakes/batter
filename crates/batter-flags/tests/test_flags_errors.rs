//! A mistake in a fixture is refused when the fixture is built, and the
//! refusal names only declared keys.
#![allow(clippy::unwrap_used, reason = "a panic fails the test")]
#![allow(clippy::missing_assert_message, reason = "test names locate failures")]

#[path = "support/world.rs"]
mod world;

use batter_flags::{
    core::Toggle,
    test::{FixtureError, TestFlags},
};
use world::world;

#[test]
fn a_handle_from_another_catalog_is_refused_when_the_fixture_is_built() {
    let (catalog, h) = world();
    let (_other, foreign) = world();
    let error = TestFlags::builder(catalog, h.serving.clone())
        .unwrap()
        .set(&foreign.fast_path, Toggle::On)
        .build()
        .unwrap_err();
    assert!(matches!(&error, FixtureError::ForeignHandle { key } if key.as_str() == "fast_path"));
    let (catalog, h) = world();
    let error = TestFlags::builder(catalog, h.serving.clone())
        .unwrap()
        .engage(&foreign.checkout_halt)
        .build()
        .unwrap_err();
    assert!(matches!(error, FixtureError::ForeignHandle { .. }));
    let (catalog, h) = world();
    assert!(matches!(
        TestFlags::builder(catalog, h.serving.clone())
            .unwrap()
            .floor(&foreign.checkout_halt)
            .build()
            .unwrap_err(),
        FixtureError::ForeignHandle { .. }
    ));
}

#[test]
fn a_value_for_a_flag_the_kind_does_not_own_is_refused() {
    let (catalog, h) = world();
    let error = TestFlags::builder(catalog, h.serving.clone())
        .unwrap()
        .set(&h.worker_mode, Toggle::On)
        .build()
        .unwrap_err();
    assert!(matches!(&error, FixtureError::NotOwned { key } if key.as_str() == "worker_mode"));
    let (catalog, h) = world();
    assert!(matches!(
        TestFlags::builder(catalog, h.serving.clone())
            .unwrap()
            .engage(&h.export_halt)
            .build()
            .unwrap_err(),
        FixtureError::NotOwned { .. }
    ));
    // The same injection is fine as the kind that owns it.
    let (catalog, h) = world();
    assert!(
        TestFlags::builder(catalog, h.worker.clone())
            .unwrap()
            .set(&h.worker_mode, Toggle::On)
            .engage(&h.export_halt)
            .build()
            .is_ok()
    );
}

#[test]
fn injecting_one_declaration_twice_is_refused() {
    let (catalog, h) = world();
    let twice = TestFlags::builder(catalog, h.serving.clone())
        .unwrap()
        .set(&h.fast_path, Toggle::On)
        .set(&h.fast_path, Toggle::Off)
        .build()
        .unwrap_err();
    assert!(matches!(&twice, FixtureError::Repeated { key } if key.as_str() == "fast_path"));
    let (catalog, h) = world();
    assert!(matches!(
        TestFlags::builder(catalog, h.serving.clone())
            .unwrap()
            .engage(&h.checkout_halt)
            .release(&h.checkout_halt)
            .build()
            .unwrap_err(),
        FixtureError::Repeated { .. }
    ));
    let (catalog, h) = world();
    assert!(matches!(
        TestFlags::builder(catalog, h.serving.clone())
            .unwrap()
            .floor(&h.checkout_halt)
            .floor(&h.checkout_halt)
            .build()
            .unwrap_err(),
        FixtureError::Repeated { .. }
    ));
}

#[test]
fn the_first_mistake_is_the_one_reported() {
    let (catalog, h) = world();
    let (_other, foreign) = world();
    let error = TestFlags::builder(catalog, h.serving.clone())
        .unwrap()
        .set(&h.worker_mode, Toggle::On)
        .set(&foreign.fast_path, Toggle::On)
        .build()
        .unwrap_err();
    assert!(matches!(error, FixtureError::NotOwned { .. }), "{error}");
}

#[test]
fn a_process_kind_from_another_catalog_is_refused() {
    let (catalog, _h) = world();
    let (_other, foreign) = world();
    let error = TestFlags::builder(catalog, foreign.serving).unwrap_err();
    assert!(matches!(&error, FixtureError::ForeignKind { kind } if kind == "serving"));
}

#[test]
fn fixture_errors_say_what_to_change_and_name_only_declared_keys() {
    let (catalog, h) = world();
    let (_other, foreign) = world();
    let texts: Vec<String> = [
        TestFlags::builder(catalog, h.serving.clone())
            .unwrap()
            .set(&foreign.fast_path, Toggle::On)
            .build()
            .unwrap_err(),
        TestFlags::builder(world().0, h.serving.clone()).unwrap_err(),
    ]
    .iter()
    .map(ToString::to_string)
    .collect();
    assert_eq!(
        texts,
        [
            "`fast_path` belongs to another catalog",
            "process kind `serving` belongs to another catalog",
        ]
    );
    let (catalog, h) = world();
    let not_owned = TestFlags::builder(catalog, h.serving.clone())
        .unwrap()
        .set(&h.worker_mode, Toggle::On)
        .build()
        .unwrap_err();
    assert_eq!(
        not_owned.to_string(),
        "`worker_mode` is not owned by this process kind, so an injected value would never be read"
    );
    let (catalog, h) = world();
    let repeated = TestFlags::builder(catalog, h.serving.clone())
        .unwrap()
        .engage(&h.checkout_halt)
        .engage(&h.checkout_halt)
        .build()
        .unwrap_err();
    assert_eq!(repeated.to_string(), "`checkout_halt` was injected twice");
}
