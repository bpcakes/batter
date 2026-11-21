//! Validated retry timing and gate policies.
use std::time::Duration;

use runledger_core::jobs::JobContext;

/// Recheck interval before the deterministic per-job spread of up to 25%.
///
/// ```
/// use batter_flags::jobs::Recheck;
/// use std::time::Duration;
/// let recheck = Recheck::after(Duration::from_secs(30))?;
/// assert_eq!(recheck.interval(), Duration::from_secs(30));
/// # Ok::<(), batter_flags::jobs::RecheckError>(())
/// ```
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Recheck(Duration);

/// The requested interval is outside 30 seconds through one hour, inclusive.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[error("job gate recheck must be between 30 seconds and one hour")]
pub struct RecheckError;

impl Recheck {
    /// Validates the interval before a handler can be registered.
    pub fn after(interval: Duration) -> Result<Self, RecheckError> {
        if !(Duration::from_secs(30)..=Duration::from_secs(3600)).contains(&interval) {
            return Err(RecheckError);
        }
        Ok(Self(interval))
    }

    /// The requested interval, before per-job spreading.
    pub fn interval(self) -> Duration {
        self.0
    }

    pub(super) fn delay(self, context: &JobContext) -> Duration {
        // Fixed FNV-1a over every identity byte: no process-random hash seed and
        // no dependence on run/attempt, so retries of one job retain its spread.
        let hash = context
            .job_id
            .as_bytes()
            .iter()
            .fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
                (h ^ u64::from(*b)).wrapping_mul(0x0000_0100_0000_01b3)
            });
        self.0 + self.0.mul_f64((hash % 10_001) as f64 / 40_000.0)
    }
}

/// Outcome for a tripped switch on an application-declared direct job type.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DirectWhenTripped {
    /// Continue the logical job later; resets attempts only once committed.
    Defer(Recheck),
    /// Retry later, consuming the native attempt budget.
    DeferAttempt(Recheck),
    /// Terminal failure, delivered to the inner dead-letter hook by Runledger.
    Drop,
}

/// Outcomes allowed for a workflow step: continuation is deliberately absent.
///
/// ```compile_fail
/// use batter_flags::jobs::StepWhenTripped;
/// let policy = StepWhenTripped::Defer;
/// ```
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StepWhenTripped {
    /// Retry later, consuming the native attempt budget.
    DeferAttempt(Recheck),
    /// Terminal failure; native workflow propagation still applies.
    Drop,
}

/// Explicit policies for an operator trip and for missing initial publication.
///
/// Both fields are required. Missing publication rechecks after `awaiting`
/// whatever `when_tripped` says: a direct job continues without spending an
/// attempt, and a workflow step, which cannot continue, spends one.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GatePolicy<P> {
    /// What an actual trip does.
    pub when_tripped: P,
    /// Validated recheck for the initial publication window.
    pub awaiting: Recheck,
}
