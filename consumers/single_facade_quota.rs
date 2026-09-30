//! Application expectations retain the complete unexpected native quota outcome.

use super::Outcome;
use batter::runlimit::{
    RunResult,
    native::{Denial, QuotaDenial},
    postgres::BatchCheckError,
};
use std::{convert::Infallible, error::Error, fmt};

type QuotaOutcome = RunResult<(), Infallible, BatchCheckError>;

/// The result is inspectable explicitly, but never printed by this report.
pub(super) struct UnexpectedQuotaOutcome {
    expected: &'static str,
    pub(super) result: QuotaOutcome,
}

impl fmt::Display for UnexpectedQuotaOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.expected)
    }
}

impl fmt::Debug for UnexpectedQuotaOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UnexpectedQuotaOutcome")
            .field("expected", &self.expected)
            .finish_non_exhaustive()
    }
}

impl Error for UnexpectedQuotaOutcome {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match &self.result {
            RunResult::Backend { error, .. } => Some(error),
            RunResult::Interrupted { reason, .. } => Some(reason),
            RunResult::Admitted {
                work: Err(error), ..
            } => Some(error),
            RunResult::Admitted { work: Ok(()), .. } | RunResult::Rejected { .. } => None,
        }
    }
}

pub(super) fn expect_admission(result: QuotaOutcome) -> Outcome {
    match result {
        RunResult::Admitted { work: Ok(()), .. } => Ok(()),
        result => Err(Box::new(UnexpectedQuotaOutcome {
            expected: "first check must admit",
            result,
        })),
    }
}

pub(super) fn expect_exhaustion(result: QuotaOutcome) -> Outcome<QuotaDenial> {
    match result {
        RunResult::Rejected {
            index: 0,
            denial: Denial::QuotaExceeded(exhausted),
            ..
        } => Ok(exhausted),
        result => Err(Box::new(UnexpectedQuotaOutcome {
            expected: "exhausted check must deny with a native quota denial",
            result,
        })),
    }
}

#[cfg(test)]
#[path = "single_facade_quota_tests.rs"]
mod tests;
