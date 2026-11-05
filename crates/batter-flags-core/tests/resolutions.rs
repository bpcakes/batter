//! What a base resolved to, declaration by declaration.
#![allow(clippy::unwrap_used, reason = "a panic fails the test")]
#![allow(clippy::missing_assert_message, reason = "test names locate failures")]

#[path = "support/fixtures.rs"]
mod fixtures;
#[path = "support/world.rs"]
mod world;

use batter_flags_core::{Kind, Owners, low_level::ResolutionSource};
use world::{Literal, base, world};

#[test]
fn each_resolution_carries_the_declaration_it_resolved() {
    let (catalog, h) = world();
    let resolved = base(catalog, &h.serving, &[Literal::Flag("fast_path", "on")]);
    let resolutions: Vec<_> = resolved.resolved().collect();
    assert_eq!(resolutions.len(), resolved.catalog().declarations().count());

    for resolution in &resolutions {
        // The declaration is the one the key names, with no lookup in between.
        let declared = resolution.declared();
        assert_eq!(declared.key(), resolution.key());
        let looked_up = resolved.catalog().declaration(resolution.key().as_str());
        assert_eq!(
            looked_up.map(|found| found.fingerprint()),
            Some(declared.fingerprint())
        );
    }

    let find = |key: &str| {
        resolutions
            .iter()
            .find(|resolution| resolution.key().as_str() == key)
            .unwrap()
    };
    let fast_path = find("fast_path");
    assert_eq!(fast_path.declared().kind(), Kind::Release);
    assert_eq!(fast_path.source(), ResolutionSource::Literal);
    assert!(matches!(fast_path.declared().owners(), Owners::All));
    let worker_mode = find("worker_mode");
    assert!(
        matches!(worker_mode.declared().owners(), Owners::Only(kinds) if kinds == [h.worker.clone()])
    );
    assert_eq!(worker_mode.source(), ResolutionSource::Absent);
    assert_eq!(find("checkout_halt").declared().kind(), Kind::Switch);
}

#[test]
fn resolutions_come_in_key_order() {
    let (catalog, h) = world();
    let resolved = base(catalog, &h.worker, &[]);
    let keys: Vec<String> = resolved
        .resolved()
        .map(|resolution| resolution.key().to_string())
        .collect();
    let mut sorted = keys.clone();
    sorted.sort();
    assert_eq!(keys, sorted);
}
