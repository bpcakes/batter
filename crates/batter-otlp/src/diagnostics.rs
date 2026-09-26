//! Application-root metrics export outcomes, retained beside the service result.
//!
//! These values describe best-effort diagnostics. They never replace the
//! original service result, its report or its exit classification; see
//! [`batter_core::service::ServiceCompletion`]. Every field is a closed category or a
//! bounded count, so formatting cannot disclose collector bodies, URLs,
//! credentials or native error text.
//!
//! Collector acknowledgement claims only the inspected HTTP response for one
//! request. It does not prove downstream durable storage, and a timed-out or
//! cancelled request proves neither delivery nor remote rollback.
//!
//! ```
//! use batter_otlp::diagnostics::{ExportOutcome, MetricsExport};
//!
//! fn final_export_acknowledged(metrics: &MetricsExport) -> bool {
//!     match metrics {
//!         MetricsExport::Exported(report) => report.final_export == ExportOutcome::Acknowledged,
//!         MetricsExport::Disabled
//!         | MetricsExport::InstallationRejected { .. }
//!         | MetricsExport::Abandoned => false,
//!     }
//! }
//! # assert!(!final_export_acknowledged(&MetricsExport::Disabled));
//! ```

/// Metrics export outcome for one serving run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MetricsExport {
    /// No collector endpoint was configured, so no recorder was installed and
    /// no collector was contacted.
    Disabled,
    /// Another process-wide recorder already existed. The rejected recorder and
    /// the prepared pipeline were explicitly closed, and the service ran
    /// without this export.
    InstallationRejected {
        /// Closure of the rejected diagnostic resources.
        closure: DiagnosticClosure,
    },
    /// The recorder was installed and the export owner finalized it.
    Exported(ExportReport),
    /// The orchestration terminated without diagnostic finalization. Retained
    /// diagnostic resources were not explicitly closed.
    Abandoned,
}

/// Retained evidence from an installed export pipeline.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportReport {
    /// Bounded history of the periodic exports attempted while the service ran.
    pub periodic: ExportHistory,
    /// The export of the final snapshot, collected only after the service result
    /// and its cleanup were retained.
    pub final_export: ExportOutcome,
    /// Whether the retained service report supports complete final coverage.
    pub coverage: FinalCoverage,
    /// Explicit exporter and provider closure after the final export.
    pub closure: DiagnosticClosure,
    /// Observations rejected before the metrics bridge allocated anything.
    pub rejected: GuardRejections,
}

/// Bounded periodic export history. Only counts and the first and last failure
/// are retained; no request, sample or response queue exists.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ExportHistory {
    /// Periodic collection and export attempts.
    pub attempts: u64,
    /// Attempts whose collector response was decoded without rejected points.
    pub acknowledged: u64,
    /// Attempts that ended in an [`ExportFailure`].
    pub failed: u64,
    /// Interval ticks skipped because the previous attempt was still settling.
    /// The next attempt exports the cumulative state, so no sample is queued.
    pub coalesced_intervals: u64,
    /// First retained failure.
    pub first_failure: Option<ExportFailure>,
    /// Most recent retained failure.
    pub last_failure: Option<ExportFailure>,
}

/// Result of one collection and export attempt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportOutcome {
    /// The collector returned a success status and an OTLP response that
    /// decoded without rejected data points.
    Acknowledged,
    /// The attempt did not establish acknowledgement.
    Failed(ExportFailure),
}

/// Why one export attempt was not acknowledged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportFailure {
    /// The SDK snapshot could not be collected.
    Collection,
    /// The upstream exporter failed before invoking the bounded transport.
    Encoding,
    /// The encoded request exceeded the fixed payload ceiling; it was not sent.
    PayloadTooLarge,
    /// The connection failed before the request could be written.
    Refused,
    /// The attempt deadline expired before response headers arrived. The
    /// request may have reached the collector.
    HeadersTimedOut,
    /// The attempt deadline expired while reading the response body.
    BodyTimedOut,
    /// Another transport failure after the request may have been dispatched.
    Transport,
    /// The collector returned a non-success HTTP status.
    Status(u16),
    /// The response body exceeded the fixed response ceiling.
    ResponseTooLarge,
    /// The response body was not a valid OTLP metrics response.
    MalformedResponse,
    /// OTLP partial success: the collector rejected some data points. The
    /// request is never retried automatically.
    PartiallyRejected {
        /// Rejected data points reported by the collector.
        rejected_points: u64,
    },
}

pub use batter_core::service::{CompletionCoverage as FinalCoverage, IncompleteCoverage};

/// Explicit closure of the exporter and meter provider.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DiagnosticClosure {
    /// Exporter transport release.
    pub exporter: Closure,
    /// Meter provider and reader shutdown.
    pub provider: Closure,
}

/// Outcome of closing one diagnostic resource.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Closure {
    /// Closed by this call.
    Closed,
    /// The resource reported that it was already closed.
    AlreadyClosed,
    /// The resource reported a closure failure; its text is not retained.
    Failed,
}

/// Observations rejected by the catalog guard before the bridge registry,
/// callbacks or metadata allocated anything.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GuardRejections {
    /// Names outside the foundation catalog.
    pub unknown_names: u64,
    /// Catalog names with a label shape or value outside their vocabulary.
    pub invalid_labels: u64,
    /// Descriptions for another kind, without a unit, or above the size bound.
    pub invalid_descriptions: u64,
    /// Gauges and catalog names registered as the wrong kind.
    pub unsupported_kinds: u64,
    /// Valid keys beyond the fixed complete-key capacity.
    pub capacity: u64,
}
