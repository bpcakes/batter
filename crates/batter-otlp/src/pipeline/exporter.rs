//! The only crate-visible owner of native export and attempt authority.

use super::{BoundedClient, ResponseProfile, attempt::Owner};
use crate::diagnostics::ExportOutcome;
use batter_core::settings::SettingsError;
use opentelemetry_otlp::{MetricExporter, Protocol, WithExportConfig, WithHttpConfig};
use opentelemetry_sdk::{
    error::OTelSdkResult,
    metrics::{Temporality, data::ResourceMetrics, exporter::PushMetricExporter},
};
use std::time::Duration;
use tokio::time::Instant;

pub(crate) struct Exporter {
    native: MetricExporter,
    owner: Owner,
}

impl Exporter {
    pub(crate) fn new(
        endpoint: &url::Url,
        timeout: Duration,
        profile: ResponseProfile,
    ) -> Result<Self, SettingsError> {
        let owner = Owner::default();
        let client = BoundedClient::new(owner.gate(), profile).map_err(|error| {
            SettingsError::new("metrics.endpoint", "native client construction failed")
                .with_cause(error)
        })?;
        let native = MetricExporter::builder()
            .with_temporality(Temporality::Cumulative)
            .with_http()
            .with_http_client(client)
            .with_endpoint(endpoint.as_str())
            .with_protocol(Protocol::HttpBinary)
            .with_timeout(timeout)
            .build()
            .map_err(|error| {
                SettingsError::new("metrics.endpoint", "native exporter construction failed")
                    .with_cause(error)
            })?;
        Ok(Self { native, owner })
    }

    pub(crate) async fn export(
        &mut self,
        snapshot: &ResourceMetrics,
        deadline: Instant,
    ) -> ExportOutcome {
        self.owner
            .run(deadline, || self.native.export(snapshot))
            .await
    }

    pub(crate) fn shutdown(self) -> OTelSdkResult {
        self.native.shutdown()
    }
}
