//! Service completion -> final snapshot export -> diagnostic closure.

use super::{ServiceCompletion, serve};
use crate::{
    config::{PreparedMetrics, ServingParts},
    diagnostics::{FinalCoverage, IncompleteCoverage, MetricsExport},
};
use batter::{
    BoxError,
    cleanup::{CleanupOutcome, CleanupReport},
    lifecycle::{ShutdownFailure, ShutdownReport},
    startup::{InitializationError, StartupError},
};

/// The retained service result plus what its report can claim about coverage.
pub(super) struct ServiceEvidence {
    pub(super) result: Result<(), BoxError>,
    #[cfg_attr(not(feature = "metrics-export"), allow(dead_code))]
    pub(super) coverage: FinalCoverage,
}

/// One owned serving run. Diagnostics are installed before startup and
/// finalized only after the service result, including startup failure cleanup
/// or complete driver settlement, has been retained.
pub(super) async fn orchestrate(
    parts: ServingParts,
    metrics: PreparedMetrics,
) -> ServiceCompletion {
    match metrics {
        PreparedMetrics::Disabled => {
            let evidence = serve(parts).await;
            ServiceCompletion::new(evidence.result, MetricsExport::Disabled)
        }
        #[cfg(feature = "metrics-export")]
        PreparedMetrics::Otlp(prepared) => exported(parts, prepared).await,
    }
}

#[cfg(feature = "metrics-export")]
async fn exported(
    parts: ServingParts,
    prepared: Box<crate::metrics_export::Prepared>,
) -> ServiceCompletion {
    use crate::metrics_export::Installation;

    let session = match prepared.install() {
        Installation::Installed(session) => session,
        Installation::Rejected(closure) => {
            let evidence = serve(parts).await;
            return ServiceCompletion::new(
                evidence.result,
                MetricsExport::InstallationRejected { closure },
            );
        }
    };
    let (evidence, report) = session
        .around(serve(parts), |evidence: &ServiceEvidence| evidence.coverage)
        .await;
    ServiceCompletion::new(evidence.result, MetricsExport::Exported(report))
}

fn incomplete(coverage: IncompleteCoverage) -> FinalCoverage {
    if coverage == IncompleteCoverage::default() {
        FinalCoverage::Reported
    } else {
        FinalCoverage::Incomplete(coverage)
    }
}

fn cleanup_coverage(cleanup: &CleanupReport, mut coverage: IncompleteCoverage) -> FinalCoverage {
    coverage.skipped_cleanup = cleanup.skipped.len();
    coverage.unjoined_cleanup = cleanup
        .records
        .iter()
        .filter(|record| record.outcome == CleanupOutcome::Unjoined)
        .count();
    incomplete(coverage)
}

/// Startup failure cleanup is retained before this is called.
pub(super) fn startup_coverage<E>(error: &StartupError<InitializationError<E>>) -> FinalCoverage {
    match error {
        StartupError::Failed(failure) => {
            cleanup_coverage(&failure.cleanup, IncompleteCoverage::default())
        }
        StartupError::Coordinator(_) => FinalCoverage::Incomplete(IncompleteCoverage {
            missing_report: true,
            ..IncompleteCoverage::default()
        }),
    }
}

/// Driver return does not prove that unjoined or native-uncertain work stopped.
pub(super) fn report_coverage(report: &ShutdownReport) -> FinalCoverage {
    cleanup_coverage(
        &report.cleanup,
        IncompleteCoverage {
            unjoined_tasks: report.unjoined.len(),
            uncertain_native: report
                .managed
                .iter()
                .filter(|record| !record.outcome.allows_dependency_cleanup())
                .count(),
            ..IncompleteCoverage::default()
        },
    )
}

pub(super) fn shutdown_failure_coverage(failure: &ShutdownFailure) -> FinalCoverage {
    match failure {
        ShutdownFailure::Report(report) => report_coverage(report),
        ShutdownFailure::Coordinator(_) => FinalCoverage::Incomplete(IncompleteCoverage {
            missing_report: true,
            ..IncompleteCoverage::default()
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use batter::{
        cleanup::{CleanupRecord, SkipReason, SkippedCleanup},
        lifecycle::ShutdownCause,
        startup::{StartupCause, StartupFailure},
    };
    use std::sync::Arc;

    fn cleanup(outcomes: &[CleanupOutcome], skipped: usize) -> CleanupReport {
        CleanupReport {
            records: outcomes
                .iter()
                .map(|outcome| CleanupRecord {
                    name: "resource",
                    outcome: *outcome,
                    error: None,
                })
                .collect(),
            skipped: (0..skipped)
                .map(|_| SkippedCleanup {
                    name: "dependency",
                    reason: SkipReason::PreviousCleanupUnjoined,
                })
                .collect(),
        }
    }

    fn report(unjoined: Vec<&'static str>, cleanup: CleanupReport) -> ShutdownReport {
        ShutdownReport {
            cause: ShutdownCause::Requested,
            tasks: Vec::new(),
            managed: Vec::new(),
            completed_process_tasks: 0,
            forced_cancellation: !unjoined.is_empty(),
            abort_requested: unjoined.clone(),
            unjoined,
            cleanup,
        }
    }

    #[test]
    fn only_fully_observed_reports_claim_complete_coverage() {
        let observed = [
            CleanupOutcome::Succeeded,
            CleanupOutcome::Failed,
            CleanupOutcome::TimedOut,
        ];
        assert_eq!(
            report_coverage(&report(Vec::new(), cleanup(&observed, 0))),
            FinalCoverage::Reported
        );
        assert_eq!(
            report_coverage(&report(
                vec!["worker"],
                cleanup(&[CleanupOutcome::Unjoined], 1)
            )),
            FinalCoverage::Incomplete(IncompleteCoverage {
                unjoined_tasks: 1,
                skipped_cleanup: 1,
                unjoined_cleanup: 1,
                ..IncompleteCoverage::default()
            })
        );
        let failure = |cleanup| {
            StartupError::<InitializationError<std::io::Error>>::Failed(Arc::new(StartupFailure {
                stage: "resource.acquire",
                cause: StartupCause::Draining,
                destruction_panic: None,
                cleanup,
            }))
        };
        assert_eq!(
            startup_coverage(&failure(cleanup(&[CleanupOutcome::Succeeded], 0))),
            FinalCoverage::Reported
        );
        assert_eq!(
            startup_coverage(&failure(cleanup(&[], 2))),
            FinalCoverage::Incomplete(IncompleteCoverage {
                skipped_cleanup: 2,
                ..IncompleteCoverage::default()
            })
        );
    }
}
