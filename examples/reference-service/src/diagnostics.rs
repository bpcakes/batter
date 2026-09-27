//! Diagnostic outcomes retained separately from application exit policy.
#[cfg(feature = "metrics-export")]
pub use batter::otlp::diagnostics::{
    Closure, DiagnosticClosure, ExportFailure, ExportHistory, ExportOutcome, ExportReport,
    FinalCoverage, GuardRejections, IncompleteCoverage,
};

/// Application metrics status, including an absent endpoint or a diagnostic task
/// that ended without an adapter report.
///
/// ```
/// use batter_example_reference_service::diagnostics::MetricsExport;
///
/// let status = MetricsExport::Disabled;
/// assert!(matches!(status, MetricsExport::Disabled));
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MetricsExport {
    /// No collector endpoint was selected, so no recorder was installed.
    Disabled,
    /// A process-wide recorder was already installed; this adapter was closed.
    #[cfg(feature = "metrics-export")]
    InstallationRejected {
        /// Closure of the rejected adapter's resources.
        closure: DiagnosticClosure,
    },
    /// The adapter installed and finalized its export pipeline.
    #[cfg(feature = "metrics-export")]
    Exported(ExportReport),
    /// Diagnostic installation or task execution ended without a report.
    Abandoned,
}

#[cfg(feature = "metrics-export")]
impl From<batter::otlp::diagnostics::MetricsExport> for MetricsExport {
    fn from(report: batter::otlp::diagnostics::MetricsExport) -> Self {
        match report {
            batter::otlp::diagnostics::MetricsExport::InstallationRejected { closure } => {
                Self::InstallationRejected { closure }
            }
            batter::otlp::diagnostics::MetricsExport::Exported(report) => Self::Exported(report),
        }
    }
}
