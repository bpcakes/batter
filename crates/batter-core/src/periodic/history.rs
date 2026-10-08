//! Bounded, library-owned recoverable-failure evidence.
//!
//! The history lives behind an [`Arc`] that the supervisor retains at
//! registration, independently of the runner. Earlier evidence therefore
//! survives a later panic, abort or lost waiter, and the completion report
//! includes it without a second application join or observer polling.

#[cfg(test)]
mod tests;

use std::{
    error::Error,
    fmt,
    sync::{Arc, Mutex, MutexGuard},
};

/// One retained recoverable failure and the invocation that produced it.
///
/// The cause keeps its concrete type behind `dyn Error`, so the inherent
/// `downcast_ref` on `dyn Error + Send + Sync` still recovers it. Nothing here
/// requires `E: Clone` or converts the error to text. Debug never formats the
/// cause.
#[derive(Clone)]
pub struct PeriodicFailureSample {
    /// One-based index of the admitted run that failed.
    pub invocation: u64,
    /// Original application error, retained by reference count.
    pub error: Arc<dyn Error + Send + Sync>,
}

impl fmt::Debug for PeriodicFailureSample {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PeriodicFailureSample")
            .field("invocation", &self.invocation)
            .field("error", &"retained")
            .finish()
    }
}

/// How a periodic runner's loop ended, as published by the runner itself.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PeriodicCompletion {
    /// The runner published no final snapshot: it was never polled, or it
    /// panicked, was aborted, or remained unjoined. This makes no termination
    /// claim about the loop or about any remote effect it may have started.
    #[default]
    Pending,
    /// Drain was observed while initialization was still pending, so the loop
    /// abandoned its registration. Readiness stays gated on this component.
    AbandonedDuringStartup,
    /// The selected first-success allowance expired before any run succeeded.
    /// The matching [`super::PeriodicInitializationExpired`] error is retained
    /// in the process report's task records.
    InitializationExpired,
    /// The loop reached its applicable stopping point.
    Stopped,
    /// A run returned an explicit fatal failure; the original error is retained
    /// in the process report's task records.
    Fatal,
}

/// Bounded evidence about one periodic component's runs.
///
/// Counters saturate instead of wrapping, and saturation is reported rather
/// than hidden. Exactly one classified counter advances per admitted run, so a
/// retained failure is never counted twice. Later success never erases earlier
/// evidence. Only the library-owned sample count is bounded here: application
/// error payload size and caller-retained clones remain the caller's own.
///
/// `invocations` counts every admitted run. The classified counters plus at
/// most one fatal run, indicated by `completion`, account for all of them.
#[derive(Clone, Debug, Default)]
#[must_use = "inspect the retained periodic evidence"]
pub struct PeriodicSummary {
    /// Admitted runs, including the run that ended the loop.
    pub invocations: u64,
    /// Runs that returned success.
    pub succeeded: u64,
    /// Runs that returned a recoverable application failure.
    pub recoverable_failures: u64,
    /// Runs whose own deadline expired before they returned.
    pub deadline_exceeded: u64,
    /// Runs destroyed by the applicable stopping point or forced cancellation.
    pub stop_interrupted: u64,
    /// Recoverable failures whose cause was deliberately not retained, because
    /// the two bounded sample slots were already occupied.
    pub unsampled_failures: u64,
    /// Whether any counter reached its maximum and stopped advancing.
    pub saturated: bool,
    /// First retained recoverable failure, kept for the whole lifetime.
    pub first_failure: Option<PeriodicFailureSample>,
    /// Most recent retained recoverable failure, replaced as newer ones arrive.
    pub last_failure: Option<PeriodicFailureSample>,
    /// Whether this component acknowledged its registered startup.
    pub acknowledged: bool,
    /// How the loop ended, or `Pending` when the runner published nothing.
    pub completion: PeriodicCompletion,
}

impl PeriodicSummary {
    /// Whether the runner published a final snapshot. A `false` value marks an
    /// explicitly incomplete snapshot and claims nothing about termination.
    pub fn is_complete(&self) -> bool {
        self.completion != PeriodicCompletion::Pending
    }

    /// Whether any run failed, expired or was interrupted, or the component
    /// ended terminally.
    ///
    /// A fatal run and an expired initialization allowance advance no
    /// recurring counter, so both are reported here through `completion`.
    /// Abandoning pending initialization on drain is expected and is not a
    /// failure.
    pub fn has_failures(&self) -> bool {
        self.recoverable_failures != 0
            || self.deadline_exceeded != 0
            || self.stop_interrupted != 0
            || matches!(
                self.completion,
                PeriodicCompletion::Fatal | PeriodicCompletion::InitializationExpired
            )
    }
}

/// One periodic component's name and its retained evidence at report time.
#[derive(Clone, Debug)]
pub struct PeriodicRecord {
    /// Validated registration name.
    pub name: &'static str,
    /// Evidence retained independently of the runner.
    pub summary: PeriodicSummary,
}

/// Read-only access to one periodic component's retained evidence.
///
/// This projection owns no work and no history: it cannot run, stop, cancel or
/// clear anything, and dropping it changes nothing. Reading is never required,
/// because the completion report already carries the same evidence.
///
/// ```compile_fail,E0599
/// use batter_core::periodic::PeriodicReader;
/// fn cannot_run(reader: PeriodicReader) {
///     reader.run();
/// }
/// ```
///
/// ```compile_fail,E0599
/// use batter_core::periodic::PeriodicReader;
/// fn cannot_clear(reader: PeriodicReader) {
///     reader.clear();
/// }
/// ```
#[derive(Clone)]
pub struct PeriodicReader {
    name: &'static str,
    history: Arc<History>,
}

impl PeriodicReader {
    /// Validated registration name of the observed component.
    pub fn name(&self) -> &'static str {
        self.name
    }

    /// Copy the current evidence without waiting or changing ownership.
    pub fn snapshot(&self) -> PeriodicSummary {
        self.history.snapshot()
    }
}

impl fmt::Debug for PeriodicReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PeriodicReader")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

/// The supervisor's independent retention of one component's history.
pub(crate) struct RetainedHistory {
    name: &'static str,
    history: Arc<History>,
}

impl RetainedHistory {
    pub(super) fn new(name: &'static str, history: Arc<History>) -> Self {
        Self { name, history }
    }

    /// Freeze the evidence for the completion report, reconciling the runner's
    /// own marker with the coordinator's join evidence.
    ///
    /// A runner that never published leaves `completion` at `Pending`. So does
    /// one the coordinator could not join: a published marker is the loop's
    /// claim about itself, and a runner whose task was never observed cannot
    /// support a termination claim even when it wrote one first.
    pub(crate) fn into_record(self, unjoined: &[&'static str]) -> PeriodicRecord {
        let mut summary = self.history.snapshot();
        if unjoined.contains(&self.name) {
            summary.completion = PeriodicCompletion::Pending;
        }
        PeriodicRecord {
            name: self.name,
            summary,
        }
    }
}

type Cause = Arc<dyn Error + Send + Sync>;

/// The one writer of a component's bounded evidence.
pub(super) struct History {
    state: Mutex<PeriodicSummary>,
}

impl History {
    pub(super) fn new() -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(PeriodicSummary::default()),
        })
    }

    pub(super) fn reader(self: &Arc<Self>, name: &'static str) -> PeriodicReader {
        PeriodicReader {
            name,
            history: self.clone(),
        }
    }

    fn lock(&self) -> MutexGuard<'_, PeriodicSummary> {
        self.state.lock().unwrap_or_else(|error| error.into_inner())
    }

    fn snapshot(&self) -> PeriodicSummary {
        self.lock().clone()
    }

    /// Whether the publication lock is held right now. Failure-path tests use
    /// this to prove that a displaced application error is destroyed outside it.
    #[cfg(test)]
    fn is_publishing(&self) -> bool {
        self.state.try_lock().is_err()
    }

    pub(super) fn admitted(&self) {
        let mut guard = self.lock();
        let state = &mut *guard;
        bump(&mut state.invocations, &mut state.saturated);
    }

    pub(super) fn succeeded(&self) {
        let mut guard = self.lock();
        let state = &mut *guard;
        bump(&mut state.succeeded, &mut state.saturated);
    }

    pub(super) fn deadline_exceeded(&self) {
        let mut guard = self.lock();
        let state = &mut *guard;
        bump(&mut state.deadline_exceeded, &mut state.saturated);
    }

    pub(super) fn stop_interrupted(&self) {
        let mut guard = self.lock();
        let state = &mut *guard;
        bump(&mut state.stop_interrupted, &mut state.saturated);
    }

    pub(super) fn acknowledged(&self) {
        self.lock().acknowledged = true;
    }

    pub(super) fn finished(&self, completion: PeriodicCompletion) {
        self.lock().completion = completion;
    }

    /// Retain one recoverable failure in the bounded sample slots.
    ///
    /// A displaced cause is moved out of the guarded state and destroyed only
    /// after the publication lock is released, so an application destructor can
    /// never run under it. The caller already polls and destroys this future
    /// inside the component's protected tracing dispatch.
    pub(super) fn recoverable(&self, invocation: u64, error: Cause) {
        let displaced = {
            let mut guard = self.lock();
            let state = &mut *guard;
            bump(&mut state.recoverable_failures, &mut state.saturated);
            let sample = PeriodicFailureSample { invocation, error };
            if state.first_failure.is_none() {
                state.first_failure = Some(sample);
                None
            } else {
                let displaced = state.last_failure.replace(sample);
                if displaced.is_some() {
                    bump(&mut state.unsampled_failures, &mut state.saturated);
                }
                displaced
            }
        };
        // The guard above is already released; destroy the displaced cause here.
        drop(displaced);
    }
}

fn bump(counter: &mut u64, saturated: &mut bool) {
    match counter.checked_add(1) {
        Some(next) => *counter = next,
        None => *saturated = true,
    }
}
