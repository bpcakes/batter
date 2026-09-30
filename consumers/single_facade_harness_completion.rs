//! Combine observed child execution and native database teardown outcomes.

use super::Outcome;
use batter::test_support::TestFailure;
use std::{error::Error, fmt, io, process::ExitStatus};

type Cause = Box<dyn Error + Send + Sync>;

pub(super) enum ExecutionFailure {
    Launch(io::Error),
    Exit(ExitStatus),
}

impl fmt::Debug for ExecutionFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Launch(_) => f.write_str("ExecutionFailure::Launch(..)"),
            Self::Exit(status) => f
                .debug_tuple("ExecutionFailure::Exit")
                .field(status)
                .finish(),
        }
    }
}

impl fmt::Display for ExecutionFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Launch(_) => f.write_str("single-facade consumer launch or wait failed"),
            Self::Exit(status) => write!(f, "single-facade consumer exited with {status}"),
        }
    }
}

impl Error for ExecutionFailure {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Launch(error) => Some(error),
            Self::Exit(_) => None,
        }
    }
}

/// The generic combiner retains both causes; this report redacts their formatting.
pub(super) struct HarnessFailure(pub(super) TestFailure<ExecutionFailure, Cause>);

impl fmt::Debug for HarnessFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut report = f.debug_struct("HarnessFailure");
        match &self.0 {
            TestFailure::Body(body) => report.field("execution", body),
            TestFailure::Cleanup(_) => report.field("teardown_failed", &true),
            TestFailure::Both { body, .. } => report
                .field("execution", body)
                .field("teardown_failed", &true),
        };
        report.finish_non_exhaustive()
    }
}

impl fmt::Display for HarnessFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("single-facade execution or database teardown failed")
    }
}

impl Error for HarnessFailure {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match &self.0 {
            TestFailure::Body(body) | TestFailure::Both { body, .. } => Some(body),
            TestFailure::Cleanup(cleanup) => Some(cleanup.as_ref()),
        }
    }
}

/// Call only after both the child and explicit native teardown have completed.
pub(super) fn finish<E>(status: io::Result<ExitStatus>, teardown: Result<(), E>) -> Outcome
where
    E: Error + Send + Sync + 'static,
{
    let execution = status.map_err(ExecutionFailure::Launch).and_then(|status| {
        if status.success() {
            Ok(())
        } else {
            Err(ExecutionFailure::Exit(status))
        }
    });
    let teardown = teardown.map_err(|error| Box::new(error) as Cause);
    batter::test_support::finish(execution, teardown)
        .map_err(|failure| Box::new(HarnessFailure(failure)) as Cause)
}

#[cfg(test)]
#[path = "single_facade_harness_tests.rs"]
mod tests;
