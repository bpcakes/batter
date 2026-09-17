use crate::{load, source};

#[test]
fn listener_announcement_requires_an_explicit_absolute_unix_path() {
    for invalid in [
        "",
        "relative.sock",
        "/tmp/bad\0path",
        &format!("/tmp/{}", "x".repeat(200)),
    ] {
        let error = load(&[("BATTER_LISTENER_ANNOUNCEMENT_PATH", invalid)]).unwrap_err();
        assert_eq!(error.field(), "BATTER_LISTENER_ANNOUNCEMENT_PATH");
        assert!(!format!("{error:?}").contains(invalid) || invalid.is_empty());
    }
    // Validation does not open a socket or require its destination to exist.
    assert!(
        load(&[(
            "BATTER_LISTENER_ANNOUNCEMENT_PATH",
            "/tmp/absent-fixture-socket"
        )])
        .is_ok()
    );
}
use axum::http::HeaderValue;
use batter::settings::{SettingsSource, read_literal};
use batter_example_reference_service::config::{
    MaintenanceSettings, PoolSettings, ServingSettings, WorkerSettings,
};
use std::{error::Error, io::Cursor, time::Duration};

#[test]
fn root_defaults_and_all_worker_fields_reach_native_values() {
    let root = load(&[]).unwrap();
    assert_eq!(root.bind().to_string(), "127.0.0.1:3000");
    let pool = root.pool_options();
    assert_eq!(pool.get_max_connections(), 8);
    assert_eq!(pool.get_min_connections(), 0);
    assert_eq!(pool.get_acquire_timeout(), Duration::from_secs(3));
    let jobs = root.jobs_config();
    assert_eq!(jobs.worker_id, "configured-worker");
    assert_eq!(jobs.poll_interval, Duration::from_millis(500));
    assert_eq!(jobs.claim_batch_size, 16);
    assert_eq!(jobs.lease_ttl_seconds, 60);
    assert_eq!(jobs.max_global_concurrency, 32);
    assert_eq!(jobs.reaper_interval, Duration::from_secs(15));
    assert_eq!(jobs.schedule_poll_interval, Duration::from_secs(30));
    assert_eq!(jobs.reaper_retry_delay_ms, 30_000);
    let owner = root
        .authenticator()
        .authenticate(Some(&HeaderValue::from_static(
            "Bearer fake-configured-token",
        )))
        .unwrap();
    assert_eq!(
        owner.as_uuid().to_string(),
        "00000000-0000-0000-0000-000000000001"
    );
}

#[test]
fn complete_serving_preparation_is_inert_before_runtime_transfer() {
    super::process::native("inert-serving-preparation");
}

#[test]
fn explicit_values_reach_all_native_worker_and_pool_fields() {
    let root = load(&[
        ("BATTER_BIND", "[::1]:0"),
        ("BATTER_POOL_MAX_CONNECTIONS", "3"),
        ("BATTER_POOL_MIN_CONNECTIONS", "2"),
        ("BATTER_POOL_ACQUIRE_TIMEOUT_MS", "73"),
        ("JOBS_WORKER_ID", "explicit-worker"),
        ("JOBS_POLL_INTERVAL_MS", "7"),
        ("JOBS_CLAIM_BATCH_SIZE", "9"),
        ("JOBS_LEASE_TTL_SECONDS", "1"),
        ("JOBS_MAX_GLOBAL_CONCURRENCY", "5"),
        ("JOBS_REAPER_INTERVAL_SECONDS", "11"),
        ("JOBS_SCHEDULE_POLL_INTERVAL_SECONDS", "13"),
        ("JOBS_REAPER_RETRY_DELAY_MS", "1"),
    ])
    .unwrap();
    assert_eq!(root.bind().to_string(), "[::1]:0");
    let pool = root.pool_options();
    assert_eq!(pool.get_max_connections(), 3);
    assert_eq!(pool.get_min_connections(), 2);
    assert_eq!(pool.get_acquire_timeout(), Duration::from_millis(73));
    let jobs = root.jobs_config();
    assert_eq!(jobs.worker_id, "explicit-worker");
    assert_eq!(jobs.poll_interval, Duration::from_millis(7));
    assert_eq!(jobs.claim_batch_size, 9);
    assert_eq!(jobs.lease_ttl_seconds, 1); // No from_env 10-second clamp.
    assert_eq!(jobs.max_global_concurrency, 5);
    assert_eq!(jobs.reaper_interval, Duration::from_secs(11));
    assert_eq!(jobs.schedule_poll_interval, Duration::from_secs(13));
    assert_eq!(jobs.reaper_retry_delay_ms, 1); // No from_env 1000-ms clamp.
    jobs.validate().unwrap();
}

#[test]
fn provider_origin_and_credential_policy_is_explicit() {
    for origin in [
        "https://provider.example/",
        "http://127.0.0.1:8080/",
        "http://[::1]:8080/",
    ] {
        assert!(
            load(&[("BATTER_PROVIDER_BASE_URL", origin)]).is_ok(),
            "valid provider origin was rejected: {origin}"
        );
    }
    for origin in [
        "ftp://provider.example/",
        "http://provider.example/",
        "http://localhost:8080/",
        "https://user:password@provider.example/",
        "https://provider.example/path",
        "https://provider.example/?query=1",
        "https://provider.example/#fragment",
    ] {
        let error = load(&[("BATTER_PROVIDER_BASE_URL", origin)]).unwrap_err();
        assert_eq!(error.field(), "BATTER_PROVIDER_BASE_URL");
    }
    for token in ["", "contains space", "contains\nnewline", &"x".repeat(257)] {
        let error = load(&[("BATTER_PROVIDER_TOKEN", token)]).unwrap_err();
        assert_eq!(error.field(), "BATTER_PROVIDER_TOKEN");
    }
    assert!(load(&[("BATTER_PROVIDER_TOKEN", &"x".repeat(256))]).is_ok());
}

#[test]
fn numeric_edges_are_rejected_before_native_clamping_or_truncation() {
    let year_ms = 31_536_000_000_u64;
    let capacity = tokio::sync::Semaphore::MAX_PERMITS as u64;
    for (name, maximum, minimum) in [
        ("BATTER_REQUEST_TIMEOUT_MS", year_ms, 1),
        ("BATTER_BULKHEAD_CAPACITY", capacity, 1),
        ("BATTER_PROVIDER_CAPACITY", capacity, 1),
        ("BATTER_PROCESS_CAPACITY", capacity, 1),
        ("BATTER_POOL_MAX_CONNECTIONS", u32::MAX.into(), 1),
        ("BATTER_POOL_ACQUIRE_TIMEOUT_MS", year_ms, 1),
        ("JOBS_POLL_INTERVAL_MS", year_ms, 1),
        ("JOBS_CLAIM_BATCH_SIZE", 1000, 1),
        ("JOBS_LEASE_TTL_SECONDS", i32::MAX as u64, 1),
        ("JOBS_MAX_GLOBAL_CONCURRENCY", capacity, 1),
        ("JOBS_REAPER_INTERVAL_SECONDS", 31_536_000, 1),
        ("JOBS_SCHEDULE_POLL_INTERVAL_SECONDS", 31_536_000, 1),
        ("JOBS_REAPER_RETRY_DELAY_MS", i32::MAX as u64, 1),
    ] {
        assert!(
            load(&[(name, &minimum.to_string())]).is_ok(),
            "{name} minimum"
        );
        assert!(
            load(&[(name, &maximum.to_string())]).is_ok(),
            "{name} maximum"
        );
        for invalid in [
            "0".to_owned(),
            "-1".to_owned(),
            "".to_owned(),
            (maximum + 1).to_string(),
            "18446744073709551616".to_owned(),
        ] {
            assert!(
                load(&[(name, &invalid)]).is_err(),
                "{name} accepted invalid boundary"
            );
        }
    }
    assert!(load(&[("BATTER_POOL_MIN_CONNECTIONS", "9")]).is_err());
    assert!(load(&[("BATTER_POOL_MIN_CONNECTIONS", "-1")]).is_err());
    assert!(load(&[("BATTER_POOL_MIN_CONNECTIONS", "4294967296")]).is_err());
    assert!(
        load(&[
            ("BATTER_POOL_MIN_CONNECTIONS", "4294967295"),
            ("BATTER_POOL_MAX_CONNECTIONS", "4294967295")
        ])
        .is_ok()
    );
    assert!(PoolSettings::new(0, 0, Duration::from_secs(1)).is_err());
    assert!(PoolSettings::new(1, 2, Duration::from_secs(1)).is_err());
    assert!(PoolSettings::new(1, 0, Duration::ZERO).is_err());
    assert!(PoolSettings::new(1, 0, Duration::MAX).is_err());
    let native = load(&[("JOBS_WORKER_ID", "   ")]).unwrap_err();
    assert!(
        native
            .source()
            .unwrap()
            .is::<runledger_runtime::config::JobsConfigValidationError>()
    );
    assert!(WorkerSettings::from_source(&SettingsSource::default()).is_err());

    let standalone_worker = WorkerSettings::from_source(&source(&[
        ("JOBS_WORKER_ID", "standalone-worker"),
        ("JOBS_POLL_INTERVAL_MS", &year_ms.to_string()),
    ]))
    .unwrap();
    assert_eq!(
        standalone_worker.jobs_config().poll_interval,
        Duration::from_millis(year_ms)
    );
}

#[test]
fn serving_and_maintenance_schemas_are_purpose_specific() {
    let endpoint = "postgres://user@localhost/database?sslmode=disable";
    let maintenance = MaintenanceSettings::from_sources(
        None,
        SettingsSource::default(),
        source(&[("DATABASE_URL", endpoint)]),
    )
    .unwrap();
    assert_eq!(
        format!("{maintenance:?}"),
        "MaintenanceSettings([REDACTED])"
    );
    assert!(
        ServingSettings::from_sources(
            None,
            SettingsSource::default(),
            source(&[("DATABASE_URL", endpoint), ("JOBS_WORKER_ID", "worker")])
        )
        .is_err()
    );
    for passwordless in [
        "postgres://user@localhost/database?sslmode=disable",
        "postgres://user:@localhost/database?sslmode=disable",
    ] {
        let error = load(&[("DATABASE_URL", passwordless)]).unwrap_err();
        assert_eq!(error.field(), "DATABASE_URL");
    }
    assert!(
        MaintenanceSettings::from_sources(
            None,
            SettingsSource::default(),
            source(&[("DATABASE_URL", endpoint), ("JOBS_LEASE_TTL_SECONDS", "-1")])
        )
        .is_err()
    );
    assert!(
        MaintenanceSettings::from_sources(
            None,
            SettingsSource::default(),
            source(&[("DATABASE_URL", endpoint), ("JOBS_WORKER_ID", "")])
        )
        .is_err()
    );
    let serving_endpoint = "postgres://user:fake-password@localhost/database?sslmode=disable";
    assert!(
        ServingSettings::from_sources(
            None,
            SettingsSource::default(),
            source(&[
                ("DATABASE_URL", serving_endpoint),
                ("JOBS_WORKER_ID", "worker")
            ])
        )
        .is_err()
    );
    for authentication in [
        &[(
            "BATTER_AUTH_OWNER_ID",
            "00000000-0000-0000-0000-000000000002",
        )][..],
        &[("BATTER_AUTH_TOKEN", "token-only")][..],
    ] {
        let mut setup = source(&[("DATABASE_URL", endpoint)]);
        setup.overlay(source(authentication));
        assert!(MaintenanceSettings::from_sources(None, SettingsSource::default(), setup).is_err());
    }
    assert!(
        load(&[(
            "BATTER_AUTH_OWNER_ID",
            "00000000-0000-0000-0000-000000000000"
        )])
        .is_err()
    );
    assert!(load(&[("BATTER_AUTH_TOKEN", "token with space")]).is_err());
}

#[test]
fn serving_shaped_inputs_cannot_promote_maintenance() {
    let endpoint = "postgres://user:fake@localhost/database?sslmode=disable";
    let serving_only = [
        ("BATTER_BIND", "127.0.0.1:3000"),
        (
            "BATTER_LISTENER_ANNOUNCEMENT_PATH",
            "/tmp/fixture-announcement",
        ),
        ("BATTER_REQUEST_TIMEOUT_MS", "2000"),
        ("BATTER_BULKHEAD_CAPACITY", "32"),
        ("BATTER_PROCESS_CAPACITY", "32"),
        ("BATTER_POOL_MAX_CONNECTIONS", "8"),
        (
            "BATTER_AUTH_OWNER_ID",
            "00000000-0000-0000-0000-000000000001",
        ),
        ("BATTER_AUTH_TOKEN", "fake-token"),
        ("BATTER_PROVIDER_BASE_URL", "http://127.0.0.1:9/"),
        ("BATTER_PROVIDER_TOKEN", "fake-provider-token"),
        ("BATTER_PROVIDER_CAPACITY", "32"),
        ("JOBS_WORKER_ID", "worker"),
    ];
    for pair in serving_only {
        let mut values = source(&[("DATABASE_URL", endpoint)]);
        values.overlay(source(&[pair]));
        let error =
            MaintenanceSettings::from_sources(None, SettingsSource::default(), values).unwrap_err();
        assert_eq!(error.field(), "source", "{} entered maintenance", pair.0);
    }

    let mut environment = source(&[("DATABASE_URL", endpoint)]);
    environment.overlay(source(&serving_only));
    assert!(
        MaintenanceSettings::from_sources(None, environment, SettingsSource::default()).is_ok()
    );

    let file = read_literal(
        Cursor::new(format!(
            "DATABASE_URL={endpoint}\nBATTER_BIND=127.0.0.1:3000\n"
        )),
        1024,
    )
    .unwrap();
    assert!(
        MaintenanceSettings::from_sources(
            Some(file),
            SettingsSource::default(),
            SettingsSource::default(),
        )
        .is_err()
    );
}

fn serving_values(endpoint: &str) -> SettingsSource {
    source(&[
        ("DATABASE_URL", endpoint),
        ("JOBS_WORKER_ID", "configured-worker"),
        (
            "BATTER_AUTH_OWNER_ID",
            "00000000-0000-0000-0000-000000000001",
        ),
        ("BATTER_AUTH_TOKEN", "fake-configured-token"),
        ("BATTER_PROVIDER_BASE_URL", "http://127.0.0.1:9/"),
        ("BATTER_PROVIDER_TOKEN", "fake-provider-token"),
    ])
}

fn assert_serving_unknown_key_policy(endpoint: &str) {
    assert!(
        ServingSettings::from_sources(
            None,
            source(&[("UNRELATED", "ignored")]),
            serving_values(endpoint),
        )
        .is_ok()
    );
    for name in [
        "BATTER_UNKNOWN",
        "JOBS_INTENT_PROMOTER_BATCH_SIZE",
        "PGPASSWORD",
        "PGSSLMODE",
        "PGPASSFILE",
    ] {
        assert!(
            ServingSettings::from_sources(None, source(&[(name, "")]), serving_values(endpoint),)
                .is_err(),
            "serving environment accepted {name}"
        );
        assert!(
            ServingSettings::from_sources(
                Some(source(&[(name, "")])),
                SettingsSource::default(),
                serving_values(endpoint),
            )
            .is_err(),
            "serving file accepted {name}"
        );
        let mut overrides = serving_values(endpoint);
        overrides.overlay(source(&[(name, "")]));
        assert!(
            ServingSettings::from_sources(None, SettingsSource::default(), overrides).is_err(),
            "serving overrides accepted {name}"
        );
    }
}

#[test]
fn source_order_and_unknown_key_policy_are_explicit() {
    let endpoint = "postgres://user@localhost/database?sslmode=disable";
    let file = read_literal(Cursor::new("DATABASE_URL=invalid-file-value\n"), 1024).unwrap();
    assert!(
        MaintenanceSettings::from_sources(
            Some(file.clone()),
            source(&[("DATABASE_URL", "invalid-environment-value")]),
            source(&[("DATABASE_URL", endpoint)]),
        )
        .is_ok()
    );
    assert!(
        MaintenanceSettings::from_sources(
            Some(file),
            source(&[("DATABASE_URL", endpoint)]),
            source(&[("DATABASE_URL", "invalid-winning")]),
        )
        .is_err()
    );
    assert!(
        MaintenanceSettings::from_sources(
            None,
            source(&[("UNRELATED", "ignored")]),
            source(&[("DATABASE_URL", endpoint)]),
        )
        .is_ok()
    );
    let serving_endpoint = "postgres://user:fake@localhost/database?sslmode=disable";
    let serving_file = read_literal(
        Cursor::new("DATABASE_URL=invalid-file-value\nBATTER_POOL_MAX_CONNECTIONS=1\n"),
        1024,
    )
    .unwrap();
    let serving = ServingSettings::from_sources(
        Some(serving_file),
        source(&[
            ("DATABASE_URL", serving_endpoint),
            ("BATTER_POOL_MAX_CONNECTIONS", "2"),
            ("JOBS_WORKER_ID", "environment-worker"),
            (
                "BATTER_AUTH_OWNER_ID",
                "00000000-0000-0000-0000-000000000001",
            ),
            ("BATTER_AUTH_TOKEN", "environment-token"),
            ("BATTER_PROVIDER_BASE_URL", "http://127.0.0.1:9/"),
            ("BATTER_PROVIDER_TOKEN", "environment-provider-token"),
        ]),
        source(&[("BATTER_POOL_MAX_CONNECTIONS", "5")]),
    )
    .unwrap();
    assert_eq!(serving.pool_options().get_max_connections(), 5);
    assert_serving_unknown_key_policy(serving_endpoint);
    for name in [
        "BATTER_UNKNOWN",
        "JOBS_INTENT_PROMOTER_BATCH_SIZE",
        "PGPASSWORD",
        "PGSSLMODE",
        "PGPASSFILE",
    ] {
        assert!(
            MaintenanceSettings::from_sources(
                None,
                source(&[(name, "")]),
                source(&[("DATABASE_URL", endpoint)])
            )
            .is_err()
        );
    }
    for dedicated in [false, true] {
        let (file, overrides) = if dedicated {
            (
                Some(source(&[("secret-unknown", "marker")])),
                source(&[("DATABASE_URL", endpoint)]),
            )
        } else {
            (
                None,
                source(&[("secret-unknown", "marker"), ("DATABASE_URL", endpoint)]),
            )
        };
        assert!(
            MaintenanceSettings::from_sources(file, SettingsSource::default(), overrides).is_err()
        );
    }
}
