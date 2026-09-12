use crate::{load, source};
use axum::http::HeaderValue;
use batter::settings::{SettingsSource, read_literal};
use batter_example_reference_service::config::{
    ConfigMode, PoolSettings, RootSettings, WorkerSettings,
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
    let jobs = root.worker().jobs_config().unwrap();
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
        .unwrap()
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
    let jobs = root.worker().jobs_config().unwrap();
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
fn numeric_edges_are_rejected_before_native_clamping_or_truncation() {
    let year_ms = 31_536_000_000_u64;
    let capacity = tokio::sync::Semaphore::MAX_PERMITS as u64;
    for (name, maximum, minimum) in [
        ("BATTER_REQUEST_TIMEOUT_MS", year_ms, 1),
        ("BATTER_BULKHEAD_CAPACITY", capacity, 1),
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
        standalone_worker.jobs_config().unwrap().poll_interval,
        Duration::from_millis(year_ms)
    );
}

#[test]
fn application_source_order_and_mode_requirements_are_explicit() {
    let endpoint = "postgres://user@localhost/database?sslmode=disable";
    let setup = RootSettings::from_sources(
        ConfigMode::Setup,
        None,
        SettingsSource::default(),
        source(&[("DATABASE_URL", endpoint)]),
    )
    .unwrap();
    assert!(setup.worker().jobs_config().is_err());
    assert!(setup.authenticator().is_err());
    assert!(
        RootSettings::from_sources(
            ConfigMode::Serve,
            None,
            SettingsSource::default(),
            source(&[("DATABASE_URL", endpoint), ("JOBS_WORKER_ID", "worker")])
        )
        .is_err()
    );
    assert!(
        RootSettings::from_sources(
            ConfigMode::Setup,
            None,
            SettingsSource::default(),
            source(&[("DATABASE_URL", endpoint), ("JOBS_LEASE_TTL_SECONDS", "-1")])
        )
        .is_err()
    );
    assert!(
        RootSettings::from_sources(
            ConfigMode::Setup,
            None,
            SettingsSource::default(),
            source(&[("DATABASE_URL", endpoint), ("JOBS_WORKER_ID", "")])
        )
        .is_err()
    );
    let serving_endpoint = "postgres://user:fake-password@localhost/database?sslmode=disable";
    assert!(
        RootSettings::from_sources(
            ConfigMode::Serve,
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
        assert!(
            RootSettings::from_sources(ConfigMode::Setup, None, SettingsSource::default(), setup)
                .is_err()
        );
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
fn source_order_and_unknown_key_policy_are_explicit() {
    let endpoint = "postgres://user@localhost/database?sslmode=disable";
    let file = read_literal(
        Cursor::new(format!(
            "DATABASE_URL={endpoint}\nBATTER_POOL_MAX_CONNECTIONS=invalid-shadowed\n"
        )),
        1024,
    )
    .unwrap();
    let root = RootSettings::from_sources(
        ConfigMode::Setup,
        Some(file.clone()),
        source(&[
            ("BATTER_POOL_MAX_CONNECTIONS", "2"),
            ("UNRELATED", "ignored"),
        ]),
        source(&[("BATTER_POOL_MAX_CONNECTIONS", "5")]),
    )
    .unwrap();
    assert_eq!(root.pool_options().get_max_connections(), 5);
    assert!(
        RootSettings::from_sources(
            ConfigMode::Setup,
            Some(file),
            source(&[("BATTER_POOL_MAX_CONNECTIONS", "2")]),
            source(&[("BATTER_POOL_MAX_CONNECTIONS", "invalid-winning")])
        )
        .is_err()
    );
    for name in [
        "BATTER_UNKNOWN",
        "JOBS_INTENT_PROMOTER_BATCH_SIZE",
        "PGPASSWORD",
        "PGSSLMODE",
        "PGPASSFILE",
    ] {
        assert!(
            RootSettings::from_sources(
                ConfigMode::Setup,
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
            RootSettings::from_sources(
                ConfigMode::Setup,
                file,
                SettingsSource::default(),
                overrides
            )
            .is_err()
        );
    }
}
