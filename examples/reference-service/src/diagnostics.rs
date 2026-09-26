//! Diagnostic outcomes retained separately from application exit policy.
#[cfg(feature = "metrics-export")]
pub use batter::otlp::diagnostics::*;

/// Diagnostic status when this build has no exporter capability.
#[cfg(not(feature = "metrics-export"))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MetricsExport {
    /// No recorder was installed.
    Disabled,
    /// Diagnostic execution terminated without a report.
    Abandoned,
}
