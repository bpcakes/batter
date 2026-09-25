//! Name-table, timing and series-bound unit checks.

use super::*;

#[test]
fn registration_vocabulary_is_shared_and_rejects_urls_and_error_text() {
    static TABLE: NameTable = NameTable::new("vocabulary");
    for name in ["example.read", "http-server", "Refresh", "x9_y.z"] {
        assert_eq!(TABLE.label(name), name);
    }
    let longest: &'static str = String::leak("a".repeat(MAX_NAME_LEN));
    assert_eq!(TABLE.label(longest), longest);
    let too_long: &'static str = String::leak("a".repeat(MAX_NAME_LEN + 1));
    for name in [
        "",
        too_long,
        "https://example.test/items/7",
        "/items/7",
        "user@example.test",
        "connection refused: peer reset",
        INVALID_NAME,
        OVERFLOW_NAME,
    ] {
        assert_eq!(TABLE.label(name), INVALID_NAME, "{name}");
    }
}

#[test]
fn full_table_coalesces_without_eviction() {
    static TABLE: NameTable = NameTable::new("test");
    let names: Vec<&'static str> = (0..NAME_CAPACITY + 8)
        .map(|index| &*String::leak(format!("name.n{index}")))
        .collect();
    for name in &names[..NAME_CAPACITY] {
        assert_eq!(TABLE.label(name), *name);
    }
    for name in &names[NAME_CAPACITY..] {
        assert_eq!(TABLE.label(name), OVERFLOW_NAME);
    }
    assert_eq!(TABLE.label(names[0]), names[0]);
    assert_eq!(TABLE.label("Not Valid"), INVALID_NAME);
}

#[test]
fn timing_samples_are_finite_and_nonnegative() {
    assert_eq!(seconds(Duration::ZERO), 0.0);
    let max = seconds(Duration::MAX);
    assert!(max.is_finite() && max > 0.0);
    let regressed = tokio::time::Instant::now()
        .saturating_duration_since(tokio::time::Instant::now() + Duration::from_secs(1));
    assert_eq!(seconds(regressed), 0.0);
}

#[test]
fn series_bound_matches_catalog_domains() {
    assert_eq!(
        MAX_SERIES,
        66 * 5 * 3 + 66 * 9 + 12 + 2 * 66 * 6 + 8 + 8 + 2 + 4
    );
}
