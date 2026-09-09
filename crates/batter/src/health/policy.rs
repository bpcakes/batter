use crate::{ConfigurationError, validation};
use std::time::Duration;

/// Validated sampling and observation timing, with no application defaults.
#[derive(Clone, Copy, Debug)]
pub struct HealthPolicy {
    probe_budget: Duration,
    probe_delay: Duration,
    maximum_age: Duration,
    scheduling_margin: Duration,
}

impl HealthPolicy {
    /// Validate all timings before constructing a monitor.
    ///
    /// Every duration must be positive and representable. Maximum age must be
    /// at least delay + probe budget + scheduling margin, using checked sums.
    /// This permits a normal next probe before expiration; a stalled runtime
    /// can still exceed that allowance and readers will report stale health.
    pub fn new(
        probe_budget: Duration,
        probe_delay: Duration,
        maximum_age: Duration,
        scheduling_margin: Duration,
    ) -> Result<Self, ConfigurationError> {
        validation::positive(probe_budget, "health probe budget")?;
        validation::positive(probe_delay, "health probe delay")?;
        validation::positive(maximum_age, "health maximum age")?;
        validation::positive(scheduling_margin, "health scheduling margin")?;
        let minimum_age = probe_budget
            .checked_add(probe_delay)
            .and_then(|duration| duration.checked_add(scheduling_margin))
            .ok_or(ConfigurationError::TooLarge("health combined timing"))?;
        validation::representable(minimum_age, "health combined timing")?;
        if maximum_age < minimum_age {
            return Err(ConfigurationError::InvalidHealthTiming);
        }
        Ok(Self {
            probe_budget,
            probe_delay,
            maximum_age,
            scheduling_margin,
        })
    }

    /// Total allowance for one complete native probe future.
    pub fn probe_budget(self) -> Duration {
        self.probe_budget
    }

    /// Delay after one probe has completed and been destroyed before the next.
    pub fn probe_delay(self) -> Duration {
        self.probe_delay
    }

    /// An observation is stale at this age, including exact equality.
    pub fn maximum_age(self) -> Duration {
        self.maximum_age
    }

    /// Explicit slack included in the minimum permitted maximum age.
    /// It does not delay probes or refresh observations on its own.
    pub fn scheduling_margin(self) -> Duration {
        self.scheduling_margin
    }
}
