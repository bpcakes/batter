use super::super::report::{Finding, VerificationError};
use std::task::Poll;

/// One budget shared by every phase of captured-catalog evaluation.
/// Checkpoints count object/role visits, not elapsed time. The owned operation
/// observes its cancellation and clock whenever a checkpoint returns Pending.
pub(super) struct Evaluation {
    visits: usize,
    observed_findings: usize,
    finding_bytes: usize,
}

const MAX_VISITS: usize = 1_000_000;
const MAX_FINDINGS: usize = 100_000;
const MAX_FINDING_BYTES: usize = 16 * 1024 * 1024;
const VISITS_PER_POLL: usize = 64;

impl Evaluation {
    pub(super) fn new() -> Self {
        Self {
            visits: 0,
            observed_findings: 0,
            finding_bytes: 0,
        }
    }

    pub(super) async fn checkpoint(
        &mut self,
        findings: &[Finding],
    ) -> Result<(), VerificationError> {
        self.visits += 1;
        // Expansion has no findings argument, so an empty slice does not reset
        // the accounting for the shared report assembled by other phases.
        for finding in findings.iter().skip(self.observed_findings) {
            self.finding_bytes = self
                .finding_bytes
                .saturating_add(std::mem::size_of::<Finding>())
                .saturating_add(finding.object.as_ref().map_or(0, String::len))
                .saturating_add(finding.subject.as_ref().map_or(0, String::len));
        }
        self.observed_findings = self.observed_findings.max(findings.len());
        if self.visits > MAX_VISITS
            || findings.len() > MAX_FINDINGS
            || self.finding_bytes > MAX_FINDING_BYTES
        {
            return Err(VerificationError::EvaluationCapacity);
        }
        if self.visits.is_multiple_of(VISITS_PER_POLL) {
            let mut yielded = false;
            std::future::poll_fn(|cx| {
                if yielded {
                    Poll::Ready(())
                } else {
                    yielded = true;
                    cx.waker().wake_by_ref();
                    Poll::Pending
                }
            })
            .await;
        }
        Ok(())
    }
}

#[cfg(test)]
pub(super) mod tests;
