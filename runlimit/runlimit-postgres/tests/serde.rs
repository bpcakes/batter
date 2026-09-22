//! Serde wire-contract and validation tests.

#![cfg(feature = "serde")]

use std::time::Duration;

use runlimit_postgres::PostgresConfig;
use serde_json::json;

#[test]
fn postgres_config_has_an_exact_duration_wire_shape() {
    let config = PostgresConfig::new()
        .with_maximum_rows_per_shard(257)
        .unwrap()
        .with_max_batch_size(17)
        .unwrap()
        .with_pool_acquire_timeout(Duration::new(1, 234_567))
        .unwrap()
        .with_operation_timeout(Duration::new(2, 345_678))
        .unwrap();

    let value = serde_json::to_value(config).unwrap();
    assert_eq!(
        value,
        json!({
            "maximum_rows_per_shard": 257,
            "max_batch_size": 17,
            "pool_acquire_timeout": {
                "secs": 1,
                "nanos": 234_567
            },
            "operation_timeout": {
                "secs": 2,
                "nanos": 345_678
            }
        })
    );
    assert_eq!(
        serde_json::from_value::<PostgresConfig>(value).unwrap(),
        config
    );
}

#[test]
fn postgres_config_loading_uses_constructor_defaults_and_validation() {
    let defaults: PostgresConfig = serde_json::from_value(json!({})).unwrap();
    assert_eq!(defaults, PostgresConfig::new());

    let one_nanosecond: PostgresConfig = serde_json::from_value(json!({
        "maximum_rows_per_shard": 1,
        "pool_acquire_timeout": {"secs": 0, "nanos": 1},
        "operation_timeout": {"secs": 0, "nanos": 1}
    }))
    .unwrap();
    assert_eq!(
        one_nanosecond.pool_acquire_timeout(),
        Duration::from_nanos(1)
    );
    assert_eq!(one_nanosecond.maximum_rows_per_shard(), 1);
    assert_eq!(one_nanosecond.operation_timeout(), Duration::from_nanos(1));

    for invalid in [
        json!({"maximum_rows_per_shard": 0}),
        json!({"maximum_rows_per_shard": 65_537}),
        json!({"max_batch_size": 0}),
        json!({"pool_acquire_timeout": {"secs": 0, "nanos": 0}}),
        json!({"pool_acquire_timeout": {"secs": 60, "nanos": 1}}),
        json!({"operation_timeout": {"secs": 0, "nanos": 0}}),
        json!({"operation_timeout": {"secs": 60, "nanos": 1}}),
        json!({"max_batch_size": 32, "unexpected": true}),
    ] {
        assert!(
            serde_json::from_value::<PostgresConfig>(invalid).is_err(),
            "invalid PostgreSQL configuration was accepted"
        );
    }
}
