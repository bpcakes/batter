use crate::{ConfigurationError, validation};
use std::time::Duration;

/// When a periodic component acknowledges its registered startup.
///
/// The value is opaque and validated: an allowance is rejected before
/// registration rather than at the first run. Acknowledging immediately states
/// that the loop is initialized, not that maintenance succeeded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PeriodicStartup {
    allowance: Option<Duration>,
}

impl PeriodicStartup {
    /// Acknowledge once the loop itself is initialized.
    pub fn immediate() -> Self {
        Self { allowance: None }
    }

    /// Acknowledge only after one run succeeds, within a validated positive
    /// total allowance.
    ///
    /// The allowance is measured from the component's first live execution and
    /// includes failed runs and the interval waits between them. Each
    /// initialization run is additionally capped by whichever of the run budget
    /// and this allowance expires first. Expiry before a success is a retained
    /// library initialization failure and initiates drain.
    pub fn after_first_success(allowance: Duration) -> Result<Self, ConfigurationError> {
        validation::positive(allowance, "periodic initialization allowance")?;
        Ok(Self {
            allowance: Some(allowance),
        })
    }

    /// Total initialization allowance, or `None` for immediate acknowledgement.
    pub fn initialization_allowance(self) -> Option<Duration> {
        self.allowance
    }
}

/// Which library-owned stopping class a periodic component belongs to.
///
/// There are exactly two, and [`PeriodicPolicy::new`] requires one. Neither is
/// an application-managed token, join protocol, inter-component dependency
/// graph, or a third independent shutdown timer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PeriodicShutdown {
    /// Stop admitting runs and destroy an active run when the process drains.
    StopAtDrain,
    /// Keep running bounded work during drain while ordinary direct components,
    /// admitted finite work and descendants, or native settlement still need
    /// it. The coordinator owns that stopping point; it is never later than the
    /// existing global forced-cancellation boundary.
    SupportThroughDrain,
}

/// A validated periodic schedule, per-run budget, startup and stopping class.
///
/// Timing is validated once, before registration, so an invalid schedule cannot
/// reach execution. A run budget longer than the interval is valid: it bounds
/// one run without implying overlap, because invocations remain serial.
///
/// ```
/// use batter_core::periodic::{PeriodicPolicy, PeriodicShutdown, PeriodicStartup};
/// use std::time::Duration;
///
/// let policy = PeriodicPolicy::new(
///     Duration::from_secs(30),
///     Duration::from_secs(45),
///     PeriodicStartup::immediate(),
///     PeriodicShutdown::StopAtDrain,
/// )?;
/// assert_eq!(policy.interval(), Duration::from_secs(30));
/// assert!(policy.run_budget() > policy.interval());
/// # Ok::<(), batter_core::ConfigurationError>(())
/// ```
///
/// A zero interval or budget is rejected here, not at the first tick:
///
/// ```
/// use batter_core::{ConfigurationError, periodic::{PeriodicPolicy, PeriodicShutdown, PeriodicStartup}};
/// use std::time::Duration;
///
/// assert_eq!(
///     PeriodicPolicy::new(
///         Duration::ZERO,
///         Duration::from_secs(1),
///         PeriodicStartup::immediate(),
///         PeriodicShutdown::StopAtDrain,
///     ),
///     Err(ConfigurationError::Zero("periodic interval")),
/// );
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PeriodicPolicy {
    interval: Duration,
    run_budget: Duration,
    startup: PeriodicStartup,
    shutdown: PeriodicShutdown,
}

impl PeriodicPolicy {
    /// Validate a positive, representable interval and per-run budget.
    ///
    /// Both values are also bounded by the shared representable-duration limit,
    /// so later elapsed-time and next-tick arithmetic cannot overflow or enqueue
    /// catch-up work.
    pub fn new(
        interval: Duration,
        run_budget: Duration,
        startup: PeriodicStartup,
        shutdown: PeriodicShutdown,
    ) -> Result<Self, ConfigurationError> {
        validation::positive(interval, "periodic interval")?;
        validation::positive(run_budget, "periodic run budget")?;
        Ok(Self {
            interval,
            run_budget,
            startup,
            shutdown,
        })
    }

    /// Fixed interval between invocation starts, with missed ticks skipped.
    pub fn interval(self) -> Duration {
        self.interval
    }

    /// Total budget for one run, measured from its own start.
    pub fn run_budget(self) -> Duration {
        self.run_budget
    }

    /// Selected startup acknowledgement policy.
    pub fn startup(self) -> PeriodicStartup {
        self.startup
    }

    /// Selected library-owned stopping class.
    pub fn shutdown(self) -> PeriodicShutdown {
        self.shutdown
    }
}
