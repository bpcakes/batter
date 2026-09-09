use std::{
    fmt,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::time::Instant;

/// Current state computed at a reader's snapshot, independent of process state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HealthStatus {
    /// No completed probe yet.
    Unknown,
    /// Most recent probe succeeded and remains younger than maximum age.
    Healthy,
    /// Most recent probe returned an application error.
    Failed,
    /// Most recent probe exceeded its total budget.
    TimedOut,
    /// Last observation has expired; its original outcome remains inspectable.
    Stale,
    /// The sole writer was dropped or its run ended, regardless of prior success.
    Stopped,
}

/// Retained outcome of the most recent completed probe. No history is collected.
pub enum ProbeOutcome<E> {
    /// Application probe reported success.
    Succeeded,
    /// Original concrete application error, for deliberate trusted inspection.
    Failed(Arc<E>),
    /// Deadline won; no successful dependency observation is established.
    TimedOut,
}
impl<E> Clone for ProbeOutcome<E> {
    fn clone(&self) -> Self {
        match self {
            Self::Succeeded => Self::Succeeded,
            Self::Failed(error) => Self::Failed(error.clone()),
            Self::TimedOut => Self::TimedOut,
        }
    }
}
impl<E> fmt::Debug for ProbeOutcome<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Succeeded => "Succeeded",
            Self::Failed(_) => "Failed(cause retained)",
            Self::TimedOut => "TimedOut",
        })
    }
}

/// One outcome and its monotonic completion timestamp, published together.
/// The timestamp precedes any delayed publication; reads never refresh it.
pub struct ProbeObservation<E> {
    completed_at: Instant,
    outcome: ProbeOutcome<E>,
}
impl<E> ProbeObservation<E> {
    pub(super) fn new(completed_at: Instant, outcome: ProbeOutcome<E>) -> Self {
        Self {
            completed_at,
            outcome,
        }
    }

    /// Runtime-local monotonic completion time; not a wall-clock timestamp.
    pub fn completed_at(&self) -> Instant {
        self.completed_at
    }

    /// Inspect the last result; error contents are never formatted automatically.
    pub fn outcome(&self) -> &ProbeOutcome<E> {
        &self.outcome
    }
}
impl<E> Clone for ProbeObservation<E> {
    fn clone(&self) -> Self {
        Self {
            completed_at: self.completed_at,
            outcome: self.outcome.clone(),
        }
    }
}
impl<E> fmt::Debug for ProbeObservation<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProbeObservation")
            .field("completed_at", &self.completed_at)
            .field("outcome", &self.outcome)
            .finish()
    }
}

/// A point-in-time read, not a perpetual readiness certificate.
/// Obtain a fresh snapshot for every readiness decision and also check lifecycle
/// readiness. A saved snapshot neither updates itself nor retains the writer.
pub struct HealthSnapshot<E> {
    status: HealthStatus,
    observed_at: Instant,
    last_probe: Option<ProbeObservation<E>>,
}
impl<E> HealthSnapshot<E> {
    /// Status as of this snapshot's observation instant.
    pub fn status(&self) -> HealthStatus {
        self.status
    }

    /// Whether dependency health alone was fresh and successful at this read.
    pub fn is_healthy(&self) -> bool {
        self.status == HealthStatus::Healthy
    }

    /// Monotonic instant at which freshness and writer liveness were evaluated.
    pub fn observed_at(&self) -> Instant {
        self.observed_at
    }

    /// Last completed probe, including its original result after expiry/stop.
    pub fn last_probe(&self) -> Option<&ProbeObservation<E>> {
        self.last_probe.as_ref()
    }
}
impl<E> fmt::Debug for HealthSnapshot<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HealthSnapshot")
            .field("status", &self.status)
            .field("observed_at", &self.observed_at)
            .field("last_probe", &self.last_probe)
            .finish()
    }
}

/// Cloneable, synchronous observation. Reads never invoke probes, create tasks,
/// renew timestamps, or prolong the writer's lifetime. No per-reader queue exists.
pub struct HealthReader<E> {
    publication: Arc<Publication<E>>,
}
impl<E> Clone for HealthReader<E> {
    fn clone(&self) -> Self {
        Self {
            publication: self.publication.clone(),
        }
    }
}
impl<E> HealthReader<E> {
    pub(super) fn new(publication: Arc<Publication<E>>) -> Self {
        Self { publication }
    }

    /// Read result/time and writer liveness together, calculating expiration now.
    /// A short internal mutex protects publication; no application code runs
    /// under it. The returned snapshot can become obsolete immediately.
    pub fn snapshot(&self) -> HealthSnapshot<E> {
        let published = self
            .publication
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let observed_at = Instant::now();
        let status = match (&published.last_probe, published.alive) {
            (_, false) => HealthStatus::Stopped,
            (None, true) => HealthStatus::Unknown,
            (Some(probe), true)
                if observed_at.saturating_duration_since(probe.completed_at)
                    >= self.publication.maximum_age =>
            {
                HealthStatus::Stale
            }
            (Some(probe), true) => match probe.outcome {
                ProbeOutcome::Succeeded => HealthStatus::Healthy,
                ProbeOutcome::Failed(_) => HealthStatus::Failed,
                ProbeOutcome::TimedOut => HealthStatus::TimedOut,
            },
        };
        HealthSnapshot {
            status,
            observed_at,
            last_probe: published.last_probe.clone(),
        }
    }

    /// Obtain a fresh dependency-only readiness decision, without I/O.
    pub fn is_healthy(&self) -> bool {
        self.snapshot().is_healthy()
    }
}

pub(super) struct Publication<E> {
    maximum_age: Duration,
    state: Mutex<Published<E>>,
}
struct Published<E> {
    alive: bool,
    last_probe: Option<ProbeObservation<E>>,
}
impl<E> Publication<E> {
    pub(super) fn new(maximum_age: Duration) -> Self {
        Self {
            maximum_age,
            state: Mutex::new(Published {
                alive: true,
                last_probe: None,
            }),
        }
    }

    pub(super) fn publish(&self, observation: ProbeObservation<E>) {
        let previous = {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            debug_assert!(state.alive, "only the sole live writer can publish");
            state.last_probe.replace(observation)
        };
        // Releasing the last Arc<E> may run an application destructor. Never do
        // that while holding the publication mutex needed by readers/owner Drop.
        drop(previous);
    }

    pub(super) fn stop(&self) {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .alive = false;
    }
}
