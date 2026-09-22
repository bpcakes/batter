//! Serde wire-contract and validation tests.

#![cfg(feature = "serde")]

use runlimit_memory::{MemoryStore, MemoryStoreConfig};
use serde_json::json;

#[test]
fn memory_config_has_a_complete_stable_wire_shape() {
    let config = MemoryStoreConfig::new(1_000)
        .unwrap()
        .with_shard_count(8)
        .unwrap()
        .with_max_expired_removals_per_check(5)
        .unwrap()
        .with_max_batch_size(16)
        .unwrap();

    let value = serde_json::to_value(&config).unwrap();
    assert_eq!(
        value,
        json!({
            "max_keys": 1_000,
            "shard_count": 8,
            "max_expired_removals_per_check": 5,
            "max_batch_size": 16
        })
    );
    assert_eq!(
        serde_json::from_value::<MemoryStoreConfig>(value).unwrap(),
        config
    );
}

#[test]
fn memory_config_loading_uses_constructor_defaults_and_validation() {
    let defaults: MemoryStoreConfig = serde_json::from_value(json!({"max_keys": 100})).unwrap();
    assert_eq!(defaults.max_keys(), 100);
    assert_eq!(defaults.shard_count(), 1);
    assert_eq!(defaults.max_expired_removals_per_check(), 8);
    assert_eq!(defaults.max_batch_size(), 32);

    for invalid in [
        json!({"max_keys": 0}),
        json!({"max_keys": 2, "shard_count": 3}),
        json!({"max_keys": 2, "shard_count": 0}),
        json!({"max_keys": 2, "max_expired_removals_per_check": 0}),
        json!({"max_keys": 2, "max_batch_size": 0}),
        json!({"max_keys": 2, "unexpected": true}),
    ] {
        assert!(
            serde_json::from_value::<MemoryStoreConfig>(invalid).is_err(),
            "invalid configuration was accepted"
        );
    }
}

#[test]
fn memory_stats_have_a_read_only_telemetry_shape() {
    let store = MemoryStore::new(
        MemoryStoreConfig::new(10)
            .unwrap()
            .with_shard_count(2)
            .unwrap(),
    );
    let stats = store.stats().unwrap();
    let value = serde_json::to_value(stats).unwrap();

    assert_eq!(
        value,
        json!({
            "entries": 0,
            "capacity": 10,
            "shard_count": 2
        })
    );
}
