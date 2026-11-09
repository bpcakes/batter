//! Explicit backend preparation; both profiles share aggregation and ownership.

use super::{
    CatalogRecorder, GuardState, HISTOGRAM_BOUNDS, Prepared, Resources, Schedule, SharedReader,
    config, guard,
};
use crate::transport::{AttemptSlot, BoundedClient, ResponseProfile};
use batter_core::{settings::SettingsError, telemetry::metrics::facade::KeyName};
use metrics_exporter_otel::OpenTelemetryRecorder;
use opentelemetry::{InstrumentationScope, KeyValue, metrics::MeterProvider as _};
use opentelemetry_otlp::{MetricExporter, Protocol, WithExportConfig, WithHttpConfig};
use opentelemetry_sdk::{
    Resource,
    metrics::{ManualReader, SdkMeterProvider, Temporality},
};
use std::sync::Arc;

const ENDPOINT: &str = "metrics.endpoint";

/// Reject ambient OpenTelemetry configuration at the native builder boundary.
///
/// Upstream builders read `OTEL_*` headers, endpoints, timeouts and compression
/// from the live process environment. The application must not mutate the
/// environment during preparation; this check neither reads nor echoes values.
pub fn reject_ambient_environment() -> Result<(), SettingsError> {
    if std::env::vars_os().any(|(key, _)| key.as_encoded_bytes().starts_with(b"OTEL_")) {
        return Err(SettingsError::new(
            "environment",
            "ambient OpenTelemetry settings are unsupported",
        ));
    }
    Ok(())
}

/// Prepare an inert OTLP/HTTP pipeline with explicit identity and timing.
/// Endpoint validation accepts HTTP(S) at `/v1/metrics` without credentials,
/// query or fragment. Applications may impose a narrower deployment policy.
/// Ambient `OTEL_*` is rejected because native builders merge that configuration.
/// Installation is process-global and cannot be reset or reloaded. Another
/// recorder causes a retained installation rejection and closes only this pipeline.
///
/// ```no_run
/// use batter_otlp::{prepare, Schedule};
/// use batter_core::{service, startup::ScopedStartup, startup::{ProtectedStartupScope, StartupFuture}};
/// use std::time::Duration;
/// async fn launch<F>(startup: ScopedStartup<F>) -> Result<(), batter_core::settings::SettingsError>
/// where F: for<'a> FnOnce(&'a mut ProtectedStartupScope) -> StartupFuture<'a, std::io::Error> + Send + 'static {
///     let schedule = Schedule::new(Duration::from_secs(10), Duration::from_secs(3), Duration::from_secs(5))?;
///     let exporter = prepare("http://127.0.0.1:4318/v1/metrics", "example-service", schedule)?;
///     let owner = service::start(startup, exporter);
///     let completion = owner.wait().await;
///     let _service_result = completion.service();
///     let _export_result = completion.diagnostics();
///     Ok(())
/// }
/// ```
pub fn prepare(
    endpoint: &str,
    service_name: &str,
    schedule: Schedule,
) -> Result<Prepared, SettingsError> {
    let endpoint = config::endpoint(endpoint)?;
    config::identity(service_name)?;
    reject_ambient_environment()?;
    build(
        endpoint,
        service_name,
        None,
        schedule,
        ResponseProfile::Otlp,
    )
}

/// Prepare direct VictoriaMetrics OTLP/HTTP export with a distinct producer identity.
///
/// The HTTP(S) endpoint must end in `/opentelemetry/v1/metrics`, optionally
/// preceded by nonempty path segments (for example `/insert/0` for vminsert).
/// URL credentials, query strings and fragments are rejected. Authentication
/// belongs to a trusted local collector or proxy; this API adds no auth headers.
///
/// `service_name` is 1..=64 and `instance_id` is 1..=128 ASCII letters, digits,
/// dots, underscores or hyphens. Both become resource attributes. Use a distinct
/// instance ID per concurrently exporting process, stable for its lifetime.
/// Cross-process uniqueness and backend label retention are deployment policy;
/// local validation cannot prove them. Ambient `OTEL_*` settings are rejected.
///
/// This profile accepts VictoriaMetrics' empty HTTP 200 without a Content-Type
/// header, or a valid Protobuf OTLP success response. Other statuses, nonempty
/// untyped bodies, wrong media types and partial rejections remain failures.
/// Acknowledgement does not prove durable storage. Bounds, no-retry policy and
/// protected service completion are identical to [`prepare`].
///
/// ```no_run
/// use batter_otlp::{prepare_victoriametrics, Schedule};
/// use std::time::Duration;
/// let schedule = Schedule::new(Duration::from_secs(10), Duration::from_secs(3), Duration::from_secs(5))?;
/// let diagnostics = prepare_victoriametrics(
///     "http://127.0.0.1:8428/opentelemetry/v1/metrics",
///     "example-service", "worker-1", schedule,
/// )?;
/// // Transfer diagnostics to batter_core::service::start(startup, diagnostics).
/// # Ok::<(), batter_core::settings::SettingsError>(())
/// ```
pub fn prepare_victoriametrics(
    endpoint: &str,
    service_name: &str,
    instance_id: &str,
    schedule: Schedule,
) -> Result<Prepared, SettingsError> {
    let endpoint = config::victoriametrics_endpoint(endpoint)?;
    config::identity(service_name)?;
    config::instance_identity(instance_id)?;
    reject_ambient_environment()?;
    build(
        endpoint,
        service_name,
        Some(instance_id),
        schedule,
        ResponseProfile::VictoriaMetrics,
    )
}

fn build(
    endpoint: url::Url,
    service_name: &str,
    instance_id: Option<&str>,
    schedule: Schedule,
    profile: ResponseProfile,
) -> Result<Prepared, SettingsError> {
    let slot = Arc::new(AttemptSlot::default());
    let client = BoundedClient::new(slot.clone(), profile).map_err(|error| {
        SettingsError::new(ENDPOINT, "native client construction failed").with_cause(error)
    })?;
    let exporter = MetricExporter::builder()
        .with_temporality(Temporality::Cumulative)
        .with_http()
        .with_http_client(client)
        .with_endpoint(endpoint.as_str())
        .with_protocol(Protocol::HttpBinary)
        .with_timeout(schedule.attempt)
        .build()
        .map_err(|error| {
            SettingsError::new(ENDPOINT, "native exporter construction failed").with_cause(error)
        })?;
    let reader = Arc::new(
        ManualReader::builder()
            .with_temporality(Temporality::Cumulative)
            .build(),
    );
    let mut attributes = vec![KeyValue::new("service.name", service_name.to_owned())];
    if let Some(instance_id) = instance_id {
        attributes.push(KeyValue::new("service.instance.id", instance_id.to_owned()));
    }
    let resource = Resource::builder_empty()
        .with_attributes(attributes)
        .build();
    let provider = SdkMeterProvider::builder()
        .with_resource(resource)
        .with_reader(SharedReader(reader.clone()))
        .build();
    let bridge = OpenTelemetryRecorder::new(
        provider.meter_with_scope(InstrumentationScope::builder(service_name.to_owned()).build()),
    );
    for name in guard::HISTOGRAMS {
        bridge.set_histogram_bounds(&KeyName::from_const_str(name), HISTOGRAM_BOUNDS.to_vec());
    }
    let state = Arc::new(GuardState::default());
    Ok(Prepared {
        recorder: CatalogRecorder::new(bridge, state.clone()),
        resources: Resources {
            state,
            provider,
            reader,
            exporter,
            slot,
        },
        schedule,
    })
}
