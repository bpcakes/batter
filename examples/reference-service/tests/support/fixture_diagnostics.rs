use std::{error::Error, fmt};

use batter_sqlx::test_support::FixtureReport;

pub struct ProbeError(pub Box<dyn Error + Send + Sync>, String);
impl ProbeError {
    pub fn new(error: Box<dyn Error + Send + Sync>) -> Self {
        Self(
            error,
            "fixture probe failed (cause contents redacted)".into(),
        )
    }

    /// The caller supplies only the fixed pending-phase/count summary.
    pub fn pending(error: Box<dyn Error + Send + Sync>, summary: String) -> Self {
        Self(error, summary)
    }

    pub fn report<T: Send + Sync + 'static>(report: Box<FixtureReport<T, Self>>) -> Self {
        // Only FixtureReport's deliberately redacted formatting is permitted.
        // Keep the original report for typed inspection of every native cause.
        let summary = report.to_string();
        Self(report, summary)
    }
}
impl fmt::Display for ProbeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.1)
    }
}
impl fmt::Debug for ProbeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
impl Error for ProbeError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.0.as_ref())
    }
}

/// Emit only known redacted summaries, never arbitrary native Debug/Display or
/// source-chain contents. All typed causes remain inspectable before this boundary.
#[track_caller]
pub fn assert_probe(result: Result<(), Box<dyn Error + Send + Sync>>) {
    if let Err(error) = result {
        let summary = if let Some(combined) =
            error.downcast_ref::<batter_test_support::TestFailure<ProbeError, ProbeError>>()
        {
            format!("{combined:?}")
        } else if let Some(probe) = error.downcast_ref::<ProbeError>() {
            probe.to_string()
        } else {
            "probe failed (unclassified cause contents redacted)".into()
        };
        panic!("reference probe failed: {summary}");
    }
}
