use super::ServiceOutcome;
use crate::{
    cleanup::{CleanupOutcome, CleanupReport},
    lifecycle::{ShutdownFailure, ShutdownReport},
    startup::StartupError,
};

/// Report coverage at the final diagnostic snapshot boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompletionCoverage {
    /// Every direct task and cleanup hook was observed and native settlement
    /// allowed dependency cleanup. Unsupervised producers may still exist.
    Reported,
    /// Retained evidence cannot establish complete coverage.
    Incomplete(IncompleteCoverage),
}

/// Reasons observations may be missing or arrive after the final snapshot.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct IncompleteCoverage {
    /// A coordinator terminated without its report.
    pub missing_report: bool,
    /// Direct tasks not observed to finish.
    pub unjoined_tasks: usize,
    /// Native components whose settlement did not allow dependency cleanup.
    pub uncertain_native: usize,
    /// Registered cleanup hooks never started.
    pub skipped_cleanup: usize,
    /// Cleanup hooks not observed to finish.
    pub unjoined_cleanup: usize,
}

/// Opaque proof that this service's outcome was retained before finalization.
/// It proves neither successful shutdown nor termination of arbitrary producers.
/// Obtain it only by awaiting [`super::DiagnosticCompletion::wait`].
pub struct RetainedCompletion(pub(super) CompletionCoverage);

impl RetainedCompletion {
    /// Inspect the coverage supported by the retained lifecycle reports.
    pub fn coverage(&self) -> CompletionCoverage {
        self.0
    }
}

fn cleanup(cleanup: &CleanupReport, mut coverage: IncompleteCoverage) -> CompletionCoverage {
    coverage.skipped_cleanup = cleanup.skipped.len();
    coverage.unjoined_cleanup = cleanup
        .records
        .iter()
        .filter(|record| record.outcome == CleanupOutcome::Unjoined)
        .count();
    if coverage == IncompleteCoverage::default() {
        CompletionCoverage::Reported
    } else {
        CompletionCoverage::Incomplete(coverage)
    }
}

fn shutdown(report: &ShutdownReport) -> CompletionCoverage {
    cleanup(
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

pub(super) fn coverage<E>(outcome: &ServiceOutcome<E>) -> CompletionCoverage {
    match outcome {
        ServiceOutcome::StartupFailed(StartupError::Failed(failure)) => {
            cleanup(&failure.cleanup, IncompleteCoverage::default())
        }
        ServiceOutcome::Shutdown(Ok(success)) => shutdown(success.report()),
        ServiceOutcome::Shutdown(Err(ShutdownFailure::Report(report))) => shutdown(report),
        ServiceOutcome::StartupFailed(StartupError::Coordinator(_))
        | ServiceOutcome::Shutdown(Err(ShutdownFailure::Coordinator(_)))
        | ServiceOutcome::Coordinator(_) => CompletionCoverage::Incomplete(IncompleteCoverage {
            missing_report: true,
            ..IncompleteCoverage::default()
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::CompletionCoverage as FinalCoverage;
    use super::shutdown as report_coverage;
    use super::*;
    use crate::{
        cleanup::{CleanupOutcome, CleanupReport},
        lifecycle::ShutdownReport,
        startup::{InitializationError, StartupError},
    };
    fn startup_coverage<E>(error: &StartupError<InitializationError<E>>) -> FinalCoverage {
        coverage(&ServiceOutcome::StartupFailed(error.clone()))
    }
    use crate::{
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
