//! Opt-in metrics collector settings in default and `metrics-export` builds.

use crate::{load, source};
use batter::settings::SettingsSource;
use batter_example_reference_service::config::MaintenanceSettings;

const NAME: &str = "BATTER_METRICS_OTLP_ENDPOINT";
const VALID: &str = "http://127.0.0.1:4318/v1/metrics";

fn assert_redacted(error: &batter::settings::SettingsError, field: &str) {
    assert_eq!(error.field(), field);
    let text = format!("{error} {error:?} {error:#?}");
    assert!(!text.contains("secret-marker"), "{text}");
}

#[test]
fn unconfigured_export_is_disabled_and_maintenance_ignores_the_serving_name() {
    load(&[]).expect("export is opt-in");
    // Captured environment may carry the serving-only name; dedicated maintenance
    // overrides must not.
    let endpoint = "postgres://user@localhost/database?sslmode=disable";
    MaintenanceSettings::from_sources(
        None,
        source(&[(NAME, "secret-marker"), ("DATABASE_URL", endpoint)]),
        SettingsSource::default(),
    )
    .expect("maintenance ignores serving-only environment");
    let error = MaintenanceSettings::from_sources(
        None,
        SettingsSource::default(),
        source(&[(NAME, "secret-marker"), ("DATABASE_URL", endpoint)]),
    )
    .unwrap_err();
    assert_redacted(&error, "source");
}

#[cfg(not(feature = "metrics-export"))]
#[test]
fn explicit_endpoint_without_the_feature_fails_instead_of_disabling_export() {
    for value in [VALID, "", "secret-marker"] {
        let error = load(&[(NAME, value)]).unwrap_err();
        assert_redacted(&error, NAME);
        assert_eq!(
            error.reason(),
            "metrics export is not compiled into this build"
        );
    }
    // Ambient OpenTelemetry names stay irrelevant without an explicit endpoint.
    let environment = source(&[("OTEL_EXPORTER_OTLP_HEADERS", "secret-marker")]);
    batter_example_reference_service::config::ServingSettings::from_sources(
        None,
        environment,
        crate::configuration::metrics::required(),
    )
    .expect("disabled export ignores OTEL_*");
}

pub(crate) fn required() -> SettingsSource {
    source(&[
        (
            "DATABASE_URL",
            "postgres://user:fake-password@localhost/database?sslmode=disable",
        ),
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

#[cfg(feature = "metrics-export")]
mod enabled {
    use super::*;
    use batter_example_reference_service::config::ServingSettings;

    #[test]
    fn only_literal_loopback_http_metrics_endpoints_are_accepted() {
        for value in [
            VALID,
            "http://[::1]:4318/v1/metrics",
            "http://127.0.0.2/v1/metrics",
        ] {
            load(&[(NAME, value)]).unwrap_or_else(|error| panic!("{value}: {error}"));
        }
        for value in [
            "",
            "secret-marker",
            "https://127.0.0.1:4318/v1/metrics",
            "http://localhost:4318/v1/metrics",
            "http://10.0.0.1:4318/v1/metrics",
            "http://user:secret-marker@127.0.0.1:4318/v1/metrics",
            "http://127.0.0.1:4318/v1/metrics?token=secret-marker",
            "http://127.0.0.1:4318/v1/metrics?",
            "http://127.0.0.1:4318/v1/metrics#secret-marker",
            "http://127.0.0.1:4318/v1/traces",
            "http://127.0.0.1:4318/",
        ] {
            let error = load(&[(NAME, value)]).unwrap_err();
            assert_redacted(&error, NAME);
        }
    }

    #[test]
    fn enabled_export_rejects_captured_ambient_opentelemetry_settings() {
        let mut overrides = required();
        overrides.overlay(source(&[(NAME, VALID)]));
        let environment = source(&[("OTEL_EXPORTER_OTLP_HEADERS", "secret-marker")]);
        let error = ServingSettings::from_sources(None, environment, overrides).unwrap_err();
        assert_redacted(&error, "environment");
        // Without an explicit endpoint the same ambient names are ignored.
        let environment = source(&[("OTEL_EXPORTER_OTLP_HEADERS", "secret-marker")]);
        ServingSettings::from_sources(None, environment, required())
            .expect("disabled export ignores OTEL_*");
    }

    #[test]
    fn preparation_rechecks_the_live_environment_before_native_builders() {
        super::super::process::run(
            "metrics-ambient-otel",
            &[("OTEL_EXPORTER_OTLP_HEADERS", "secret-marker".into())],
        )
        .validate_text(
            super::super::process::ExpectedExit::Code(42),
            &["configuration-child:rejected environment: ambient OpenTelemetry settings are unsupported"],
            &["secret-marker", "panicked"],
        )
        .unwrap();
    }

    /// Settings come only from injected sources; the live environment is hostile.
    pub(crate) fn ambient_child() {
        let settings = load(&[(NAME, VALID)]).expect("injected settings are valid");
        let error = batter_example_reference_service::runtime::prepare(settings).unwrap_err();
        assert_eq!(error.field(), "environment");
        eprintln!("configuration-child:rejected {error}");
        std::process::exit(42);
    }
}

#[cfg(feature = "metrics-export")]
pub(crate) use enabled::ambient_child;
