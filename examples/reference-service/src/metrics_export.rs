//! Opt-in OTLP/HTTP metrics export owned by the reference application root.
//!
//! Upstream owns aggregation and wire encoding: `metrics-exporter-otel` bridges
//! the foundation recorder calls into the OpenTelemetry SDK, a shared
//! `ManualReader` produces cumulative snapshots and the OTLP exporter encodes
//! them. This module owns only timing, limits, response classification and
//! lifecycle. One serial owner collects and exports; there is no background SDK
//! thread, `PeriodicReader`, command channel, request queue or retry loop.
//! `ManualReader::force_flush` and provider shutdown are not network flushes;
//! the final flush is an explicit collect-and-export attempt whose outcome is
//! retained before the exporter and provider are closed exactly once.

mod guard;
mod transport;

#[cfg(test)]
mod tests;

use crate::diagnostics::{
    Closure, DiagnosticClosure, ExportFailure, ExportHistory, ExportOutcome, ExportReport,
    FinalCoverage,
};
use batter::{settings::SettingsError, telemetry::metrics::facade::KeyName};
use guard::{CatalogRecorder, GuardState};
use metrics_exporter_otel::OpenTelemetryRecorder;
use opentelemetry::{InstrumentationScope, KeyValue, metrics::MeterProvider as _};
use opentelemetry_otlp::{MetricExporter, Protocol, WithExportConfig, WithHttpConfig};
use opentelemetry_sdk::{
    Resource,
    error::{OTelSdkError, OTelSdkResult},
    metrics::{
        InstrumentKind, ManualReader, Pipeline, SdkMeterProvider, Temporality,
        data::ResourceMetrics, exporter::PushMetricExporter, reader::MetricReader,
    },
};
use std::{
    sync::{Arc, Weak},
    time::Duration,
};
use tokio::{sync::watch, time::Instant};
use transport::{AttemptSlot, BoundedClient, Evidence, Phase};

/// Fixed bucket boundaries, in seconds, for both catalog histograms: twelve
/// boundaries and therefore thirteen buckets per histogram series.
pub(crate) const HISTOGRAM_BOUNDS: [f64; 12] = [
    0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0,
];
/// Instrumentation scope and the only resource attribute value.
const SERVICE_NAME: &str = "batter-example-reference-service";
const ENDPOINT: &str = "BATTER_METRICS_OTLP_ENDPOINT";

/// Fixed export timing.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Schedule {
    /// Delay between periodic attempts. Missed ticks are coalesced.
    pub(crate) interval: Duration,
    /// Deadline for one periodic collection and export attempt.
    pub(crate) attempt: Duration,
    /// Separate allowance for the final collection, export and closure. Service
    /// drain and cleanup budgets are never spent on collector I/O.
    pub(crate) final_allowance: Duration,
}

impl Schedule {
    pub(crate) const FIXED: Self = Self {
        interval: Duration::from_secs(10),
        attempt: Duration::from_secs(3),
        final_allowance: Duration::from_secs(5),
    };
}

/// Shares one manual reader between the provider and the export owner.
#[derive(Debug, Clone)]
struct SharedReader(Arc<ManualReader>);

impl MetricReader for SharedReader {
    fn register_pipeline(&self, pipeline: Weak<Pipeline>) {
        self.0.register_pipeline(pipeline);
    }

    fn collect(&self, metrics: &mut ResourceMetrics) -> OTelSdkResult {
        self.0.collect(metrics)
    }

    fn force_flush(&self) -> OTelSdkResult {
        self.0.force_flush()
    }

    fn shutdown_with_timeout(&self, timeout: Duration) -> OTelSdkResult {
        self.0.shutdown_with_timeout(timeout)
    }

    fn temporality(&self, kind: InstrumentKind) -> Temporality {
        self.0.temporality(kind)
    }
}

/// Resources closed exactly once by consuming them.
struct Resources {
    state: Arc<GuardState>,
    provider: SdkMeterProvider,
    reader: Arc<ManualReader>,
    exporter: MetricExporter,
    slot: Arc<AttemptSlot>,
}

impl Resources {
    fn close(self) -> DiagnosticClosure {
        self.state.close();
        DiagnosticClosure {
            exporter: closure(self.exporter.shutdown()),
            provider: closure(self.provider.shutdown()),
        }
    }
}

fn closure(result: OTelSdkResult) -> Closure {
    match result {
        Ok(()) => Closure::Closed,
        Err(OTelSdkError::AlreadyShutdown) => Closure::AlreadyClosed,
        Err(_) => Closure::Failed,
    }
}

/// Reject ambient OpenTelemetry configuration at the native builder boundary.
///
/// Upstream builders read `OTEL_*` headers, endpoints, timeouts and compression
/// from the live process environment. The application must not mutate the
/// environment during preparation; this check neither reads nor echoes values.
pub(crate) fn reject_ambient_environment() -> Result<(), SettingsError> {
    if std::env::vars_os().any(|(key, _)| key.as_encoded_bytes().starts_with(b"OTEL_")) {
        return Err(SettingsError::new(
            "environment",
            "ambient OpenTelemetry settings are unsupported",
        ));
    }
    Ok(())
}

/// Inert, prepared export pipeline. Nothing is installed, spawned or connected.
pub(crate) struct Prepared {
    recorder: CatalogRecorder<OpenTelemetryRecorder>,
    resources: Resources,
    schedule: Schedule,
}

/// Build the native pipeline for a validated loopback collector endpoint.
pub(crate) fn prepare(endpoint: &str, schedule: Schedule) -> Result<Prepared, SettingsError> {
    reject_ambient_environment()?;
    let slot = Arc::new(AttemptSlot::default());
    let client = BoundedClient::new(slot.clone()).map_err(|error| {
        SettingsError::new(ENDPOINT, "native client construction failed").with_cause(error)
    })?;
    let exporter = MetricExporter::builder()
        .with_temporality(Temporality::Cumulative)
        .with_http()
        .with_http_client(client)
        .with_endpoint(endpoint)
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
    let resource = Resource::builder_empty()
        .with_attributes([KeyValue::new("service.name", SERVICE_NAME)])
        .build();
    let provider = SdkMeterProvider::builder()
        .with_resource(resource)
        .with_reader(SharedReader(reader.clone()))
        .build();
    let bridge = OpenTelemetryRecorder::new(
        provider.meter_with_scope(InstrumentationScope::builder(SERVICE_NAME).build()),
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

/// Result of the one process-wide installation attempt.
pub(crate) enum Installation {
    Installed(Box<Session>),
    /// Another recorder was already installed; resources were closed.
    Rejected(DiagnosticClosure),
}

impl Prepared {
    /// Install through Batter's canonical path, which publishes the catalog
    /// descriptions after the recorder exists. Installation is process-global,
    /// happens once and cannot be reset or reloaded.
    pub(crate) fn install(self) -> Installation {
        match batter::telemetry::metrics::install(self.recorder) {
            Ok(()) => {
                Installation::Installed(Box::new(Session::new(self.resources, self.schedule)))
            }
            Err(rejected) => {
                drop(rejected.0);
                Installation::Rejected(self.resources.close())
            }
        }
    }

    #[cfg(test)]
    fn into_parts(self) -> (CatalogRecorder<OpenTelemetryRecorder>, Session) {
        (self.recorder, Session::new(self.resources, self.schedule))
    }
}

/// The single serial export owner after installation.
pub(crate) struct Session {
    resources: Resources,
    schedule: Schedule,
    snapshot: ResourceMetrics,
    history: ExportHistory,
}

impl Session {
    fn new(resources: Resources, schedule: Schedule) -> Self {
        Self {
            resources,
            schedule,
            snapshot: ResourceMetrics::default(),
            history: ExportHistory::default(),
        }
    }

    /// Collect one cumulative snapshot into the retained buffer and export it
    /// under `deadline`. Cancelling the network future proves no remote rollback.
    async fn attempt(&mut self, deadline: Instant) -> ExportOutcome {
        if self.resources.reader.collect(&mut self.snapshot).is_err() {
            return ExportOutcome::Failed(ExportFailure::Collection);
        }
        self.resources.slot.begin(deadline);
        let exported =
            tokio::time::timeout_at(deadline, self.resources.exporter.export(&self.snapshot)).await;
        classify(exported.is_err(), self.resources.slot.finish())
    }

    /// Drive `service` to completion while exporting periodically, then export
    /// the final snapshot and close the pipeline exactly once.
    ///
    /// Scheduling stops only after `service` returns; an in-flight periodic
    /// attempt settles under its own deadline before the final snapshot is
    /// collected. Nothing inside `service` can begin finalization, so readiness,
    /// drain and cleanup hooks cannot flush early. `coverage` classifies the
    /// retained service output before the final export.
    pub(crate) async fn around<T>(
        self,
        service: impl Future<Output = T>,
        coverage: impl FnOnce(&T) -> FinalCoverage,
    ) -> (T, ExportReport) {
        let (stop, stopped) = watch::channel(false);
        let (output, session) = tokio::join!(
            async move {
                let output = service.await;
                stop.send_replace(true);
                output
            },
            self.periodic(stopped),
        );
        let report = session.finish(coverage(&output)).await;
        (output, report)
    }

    /// Export periodically until `stop`; an in-flight attempt always settles
    /// under its own deadline before this returns.
    async fn periodic(mut self, mut stop: watch::Receiver<bool>) -> Self {
        let interval = self.schedule.interval;
        let mut next = Instant::now() + interval;
        loop {
            tokio::select! {
                biased;
                _ = stop.wait_for(|stopped| *stopped) => return self,
                () = tokio::time::sleep_until(next) => {}
            }
            let outcome = self.attempt(Instant::now() + self.schedule.attempt).await;
            self.history.record(outcome);
            let now = Instant::now();
            next += interval;
            while next <= now {
                next += interval;
                self.history.coalesced_intervals += 1;
            }
        }
    }

    /// Export the final snapshot under the separate allowance, then close the
    /// exporter and provider exactly once.
    async fn finish(mut self, coverage: FinalCoverage) -> ExportReport {
        let deadline = Instant::now() + self.schedule.final_allowance;
        let final_export = self.attempt(deadline).await;
        let rejected = self.resources.state.rejections();
        ExportReport {
            periodic: self.history,
            final_export,
            coverage,
            closure: self.resources.close(),
            rejected,
        }
    }
}

/// Combine the typed transport evidence with the outer deadline. Upstream
/// error text is never inspected.
fn classify(expired: bool, evidence: Evidence) -> ExportOutcome {
    match (evidence.outcome, evidence.phase) {
        (Some(Ok(())), _) => ExportOutcome::Acknowledged,
        (Some(Err(failure)), _) => ExportOutcome::Failed(failure),
        (None, Phase::Receiving) if expired => ExportOutcome::Failed(ExportFailure::BodyTimedOut),
        (None, _) if expired => ExportOutcome::Failed(ExportFailure::HeadersTimedOut),
        // Upstream finished without invoking the bounded transport.
        (None, _) => ExportOutcome::Failed(ExportFailure::Encoding),
    }
}

trait Record {
    fn record(&mut self, outcome: ExportOutcome);
}

impl Record for ExportHistory {
    fn record(&mut self, outcome: ExportOutcome) {
        self.attempts += 1;
        match outcome {
            ExportOutcome::Acknowledged => self.acknowledged += 1,
            ExportOutcome::Failed(failure) => {
                self.failed += 1;
                self.first_failure.get_or_insert(failure);
                self.last_failure = Some(failure);
            }
        }
    }
}
