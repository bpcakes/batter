//! How often the loader reads, how long a read stays fresh, and what a
//! process does before its first control-plane publication.

use std::{sync::Arc, time::Duration};

use batter_core::ConfigurationError;
use batter_flags_core::PrePublication;
use batter_flags_postgres::StoreTiming;

use super::RegisterError;
use crate::last_good::{LastGood, SnapshotDirectory};

const SECOND: Duration = Duration::from_secs(1);

/// How long before its deadline every run's store work must end: each budget
/// keeps this second of margin, so a run returns, and is judged, before the
/// periodic driver's deadline would cancel it.
pub(crate) const MARGIN: Duration = SECOND;

/// The deadline a run's store work gets: its own, less the margin.
pub(crate) fn work_deadline(deadline: tokio::time::Instant) -> tokio::time::Instant {
    deadline.checked_sub(MARGIN).unwrap_or(deadline)
}
/// The longest poll interval or maximum age a loader accepts.
const LONGEST: Duration = Duration::from_secs(3_600);

/// The loader's cadence: how often it reads, and how long a confirmed read
/// keeps views fresh. `maximum_age` is also the threshold of every
/// `StaleRule::EngageWhenStale` switch.
///
/// A poll of 15 s with a maximum age of 60 s suits most services; there is no
/// default. Registration also refuses a maximum age shorter than one poll
/// plus one run budget, so a cadence whose runs succeed never goes Stale.
///
/// ```
/// use std::time::Duration;
/// use batter_flags::control_plane::RefreshTiming;
///
/// let timing = RefreshTiming::new(Duration::from_secs(15), Duration::from_secs(60)).unwrap();
/// assert!(RefreshTiming::new(Duration::from_secs(60), Duration::from_secs(15)).is_err());
/// ```
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RefreshTiming {
    poll_interval: Duration,
    maximum_age: Duration,
}

impl RefreshTiming {
    /// Validates the cadence: both nonzero, at most an hour, and the maximum
    /// age at least the poll interval.
    pub fn new(poll_interval: Duration, maximum_age: Duration) -> Result<Self, ConfigurationError> {
        for (value, name) in [
            (poll_interval, "flags poll interval"),
            (maximum_age, "flags maximum age"),
        ] {
            if value.is_zero() {
                return Err(ConfigurationError::Zero(name));
            }
            if value > LONGEST {
                return Err(ConfigurationError::TooLarge(name));
            }
        }
        // A poll longer than the maximum age could never keep a view fresh.
        if maximum_age < poll_interval {
            return Err(ConfigurationError::TooLarge("flags poll interval"));
        }
        Ok(Self {
            poll_interval,
            maximum_age,
        })
    }

    /// How often the loader reads.
    pub fn poll_interval(self) -> Duration {
        self.poll_interval
    }

    /// How long a confirmed read keeps views fresh.
    pub fn maximum_age(self) -> Duration {
        self.maximum_age
    }
}

/// What a process serves, and whether it is ready, before its first
/// control-plane publication.
///
/// Before that publication, decisions evaluate the declared base and carry
/// `Source::NotLoaded`. Every gate reads its switch as awaiting publication:
/// gated jobs defer, gated periodic runs are skipped, and HTTP admission
/// follows the policy below.
#[derive(Clone, Debug)]
pub struct LoadPolicy {
    policy: Policy,
    last_good: LastGood,
    listen: Listen,
}

/// Whether the refresh is also woken by NOTIFY.
#[derive(Clone, Debug)]
pub(crate) enum Listen {
    /// Poll only.
    Off,
    /// A listener wakes the refresh on every committed write.
    On(Box<super::listen::ListenEndpoint>),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Policy {
    UnreadyUntilLoaded {
        timing: RefreshTiming,
        allowance: Duration,
    },
    StartOnDefaults {
        timing: RefreshTiming,
    },
}

impl LoadPolicy {
    /// The process is not ready until a control-plane read is published, and
    /// drains if none is within `allowance`. Decisions before it carry reason
    /// `error` with cause `provider_not_ready`. Registration refuses an
    /// allowance that cannot fit the first attempt and one full retry.
    pub fn unready_until_loaded(timing: RefreshTiming, allowance: Duration) -> Self {
        Self {
            policy: Policy::UnreadyUntilLoaded { timing, allowance },
            last_good: LastGood::Off,
            listen: Listen::Off,
        }
    }

    /// The process becomes ready after one bounded first attempt, whatever
    /// its outcome, and serves the declared base until a read is published.
    /// Decisions before it carry reason `default`. A root that keeps serving
    /// through a control-plane outage this way installs no flags readiness
    /// condition.
    pub fn start_on_defaults(timing: RefreshTiming) -> Self {
        Self {
            policy: Policy::StartOnDefaults { timing },
            last_good: LastGood::Off,
            listen: Listen::Off,
        }
    }

    /// Keeps the last control-plane publication in `directory`.
    ///
    /// Registration publishes a usable file before the driver starts, as
    /// `Source::PersistedFile`, so a restart during an outage serves it,
    /// engaged kill switches included. A file never makes the process ready
    /// under `unready_until_loaded` and never renews freshness: its views age
    /// from registration. Each run's budget gains a two-second file
    /// sub-deadline. Without a file, a restart during an outage loses every
    /// engagement that exists only in the control plane.
    pub fn with_last_good(self, directory: SnapshotDirectory) -> Self {
        Self {
            last_good: LastGood::File(Arc::new(directory)),
            ..self
        }
    }

    pub(crate) fn last_good(&self) -> &LastGood {
        &self.last_good
    }

    /// Also wakes the refresh when an audited write commits, through a
    /// listener on its own two-connection pool to `endpoint`. A wake never
    /// starts a refresh sooner than the endpoint's wake spacing after the
    /// previous one; the poll stays the backstop, so a lost listener only
    /// delays changes to the poll interval. LISTEN delivery is unverified
    /// until the ignored live suite runs.
    pub fn with_listen(self, endpoint: super::listen::ListenEndpoint) -> Self {
        Self {
            listen: Listen::On(Box::new(endpoint)),
            ..self
        }
    }

    pub(crate) fn listen(&self) -> &Listen {
        &self.listen
    }

    pub(crate) fn timing(&self) -> RefreshTiming {
        match self.policy {
            Policy::UnreadyUntilLoaded { timing, .. } | Policy::StartOnDefaults { timing } => {
                timing
            }
        }
    }

    pub(crate) fn pre_publication(&self) -> PrePublication {
        match self.policy {
            Policy::UnreadyUntilLoaded { .. } => PrePublication::Unready,
            Policy::StartOnDefaults { .. } => PrePublication::ServeDefaults,
        }
    }

    /// How a first failure is treated, and the startup allowance it is
    /// acknowledged within.
    pub(crate) fn startup(&self, budgets: Budgets) -> Result<Startup, RegisterError> {
        let timing = self.timing();
        let minimum_age = timing.poll_interval + budgets.refresh;
        if timing.maximum_age < minimum_age {
            return Err(RegisterError::MaximumAgeTooShort {
                minimum: minimum_age,
            });
        }
        match self.policy {
            Policy::StartOnDefaults { .. } => Ok(Startup {
                first_failure: FirstFailure::Tolerated,
                allowance: budgets.refresh + SECOND,
            }),
            Policy::UnreadyUntilLoaded { allowance, .. } => {
                let minimum = if budgets.refresh < timing.poll_interval {
                    timing.poll_interval + budgets.refresh
                } else {
                    timing.poll_interval + budgets.refresh * 2
                };
                if allowance < minimum {
                    return Err(RegisterError::AllowanceTooShort { minimum });
                }
                Ok(Startup {
                    first_failure: FirstFailure::Recoverable,
                    allowance,
                })
            }
        }
    }
}

/// How the refresh treats a failed first run.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FirstFailure {
    /// It returns `Ok`, so the run acknowledges startup whatever its outcome.
    Tolerated,
    /// It is a recoverable failure like any other.
    Recoverable,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Startup {
    pub(crate) first_failure: FirstFailure,
    pub(crate) allowance: Duration,
}

/// The run budgets, derived from the store's timeouts and each operation's
/// fixed statement count, plus a second of scheduling margin.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Budgets {
    /// One load: acquisition, then the timeouts and the read in one snapshot.
    pub(crate) refresh: Duration,
    /// One heartbeat: up to seven statements, three of them only when the
    /// catalog is published.
    pub(crate) presence: Duration,
    /// One prune: the timeouts and two deletes.
    pub(crate) prune: Duration,
}

/// The sub-deadline a run's persist gets, added to the refresh budget.
pub(crate) const FILE_SUBDEADLINE: Duration = Duration::from_secs(2);

impl Budgets {
    /// The budgets for `policy` under the store's `timing`: a policy with a
    /// last-good file adds the file sub-deadline to each refresh run.
    pub(crate) fn for_policy(timing: StoreTiming, policy: &LoadPolicy) -> Self {
        let budgets = Self::of(timing);
        match policy.last_good {
            LastGood::Off => budgets,
            LastGood::File(_) => Self {
                refresh: budgets.refresh + FILE_SUBDEADLINE,
                ..budgets
            },
        }
    }

    pub(crate) fn of(timing: StoreTiming) -> Self {
        let statements = |count: u32| timing.acquire() + timing.statement() * count + MARGIN;
        Self {
            refresh: statements(2),
            presence: statements(7),
            prune: statements(3),
        }
    }
}
