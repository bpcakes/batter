//! The refresh: one periodic component, the only loader, validator and
//! writer after registration.
//!
//! Each run reads once, then validates, publishes and records the outcome
//! synchronously after the read's last await. A deadline or drain that
//! destroys a run before then publishes nothing; a publication reached in the
//! same poll completes.

use std::{
    sync::{Arc, Mutex, PoisonError},
    time::Duration,
};

use batter_core::{
    RegistrationError,
    operation::{OperationContext, OperationError},
    periodic::{
        PeriodicFailure, PeriodicPolicy, PeriodicReader, PeriodicShutdown, PeriodicStartup,
        PeriodicWake, WakeablePolicy, register_periodic_in, register_wakeable_in,
    },
    registration::RegistrationTarget,
};
use batter_flags_core::low_level::{
    CandidateRow, Disposition, Refusal, Snapshot, Standing, ValidationScope,
};
use tokio::time::Instant;

use super::{
    LoadCause,
    source::{FetchFailure, Fetched, LoadSource},
    status::{LastRun, Shared},
    timing::{FILE_SUBDEADLINE, FirstFailure, Startup, work_deadline},
};
use crate::{
    last_good::{self, Persister},
    reader::Writer,
};

/// Everything one run needs, shared by every run of the component. The
/// writer is dropped when the component ends, which readers see as
/// `LoaderStopped`.
struct Refresh {
    source: LoadSource,
    writer: Mutex<Writer>,
    scope: ValidationScope,
    shared: Arc<Shared>,
    persistence: Persistence,
}

/// Whether each run persists what it published.
pub(crate) enum Persistence {
    Off,
    File(Persister),
}

/// A run that published nothing.
#[derive(Debug, thiserror::Error)]
#[error("flags load failed ({})", .0.as_str())]
pub(crate) struct LoadFailed(LoadCause);

/// What the next run does with a failure.
#[derive(Clone, Copy)]
enum Tolerance {
    /// The first run under `start_on_defaults`: it acknowledges startup
    /// whatever its outcome.
    FirstRun,
    /// Every later run, and every run under `unready_until_loaded`.
    Strict,
}

/// The refresh, validated and ready to register.
pub(crate) struct Prepared {
    refresh: Arc<Refresh>,
    schedule: Schedule,
    tolerance: Tolerance,
}

/// How the refresh is scheduled.
enum Schedule {
    Polled(PeriodicPolicy),
    /// Polled, and woken no sooner than its spacing after the previous run.
    Woken(WakeablePolicy),
}

/// What registering the refresh returned.
pub(crate) enum Registered {
    Polled(PeriodicReader),
    Woken(PeriodicReader, PeriodicWake),
}

impl Registered {
    pub(crate) fn reader(&self) -> PeriodicReader {
        match self {
            Self::Polled(reader) | Self::Woken(reader, _) => reader.clone(),
        }
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "crate-private assembly of one component from already validated parts"
)]
pub(crate) fn prepare(
    source: LoadSource,
    writer: Writer,
    scope: ValidationScope,
    shared: Arc<Shared>,
    persistence: Persistence,
    interval: Duration,
    budget: Duration,
    startup: Startup,
    wake_spacing: Option<Duration>,
) -> Result<Prepared, super::RegisterError> {
    let policy = PeriodicPolicy::new(
        interval,
        budget,
        PeriodicStartup::after_first_success(startup.allowance)?,
        PeriodicShutdown::StopAtDrain,
    )?;
    let schedule = match wake_spacing {
        None => Schedule::Polled(policy),
        Some(spacing) => Schedule::Woken(WakeablePolicy::new(policy, spacing)?),
    };
    Ok(Prepared {
        refresh: Arc::new(Refresh {
            source,
            writer: Mutex::new(writer),
            scope,
            shared,
            persistence,
        }),
        schedule,
        tolerance: match startup.first_failure {
            FirstFailure::Tolerated => Tolerance::FirstRun,
            FirstFailure::Recoverable => Tolerance::Strict,
        },
    })
}

impl Prepared {
    /// Registers `flags.refresh`, polled or also woken.
    pub(crate) fn register<T: RegistrationTarget + ?Sized>(
        self,
        target: &mut T,
    ) -> Result<Registered, RegistrationError> {
        let Self {
            refresh,
            schedule,
            mut tolerance,
        } = self;
        let work = move |context: OperationContext| {
            let refresh = refresh.clone();
            let this_run = std::mem::replace(&mut tolerance, Tolerance::Strict);
            async move {
                let outcome = context
                    .run("flags.load", |child: OperationContext| async move {
                        refresh.run(child.deadline()).await
                    })
                    .await;
                match (outcome, this_run) {
                    (Ok(()), _) | (Err(_), Tolerance::FirstRun) => Ok(()),
                    (Err(OperationError::Failed(failed)), Tolerance::Strict) => {
                        Err(PeriodicFailure::recoverable(failed))
                    }
                    (Err(OperationError::Interrupted(_)), Tolerance::Strict) => Err(
                        PeriodicFailure::recoverable(LoadFailed(LoadCause::Deadline)),
                    ),
                }
            }
        };
        match schedule {
            Schedule::Polled(policy) => {
                register_periodic_in(target, "flags.refresh", policy, work).map(Registered::Polled)
            }
            Schedule::Woken(policy) => register_wakeable_in(target, "flags.refresh", policy, work)
                .map(|(reader, wake)| Registered::Woken(reader, wake)),
        }
    }
}

impl Refresh {
    async fn run(&self, deadline: Instant) -> Result<(), LoadFailed> {
        let fetched = self.source.fetch(work_deadline(deadline)).await;
        // Everything below is synchronous: a run either publishes in this
        // poll or not at all.
        let (previous, result) = match fetched {
            Err(FetchFailure::SchemaBehind { found }) => (
                self.shared.schema_behind(found),
                Err(LoadCause::SchemaBehind),
            ),
            Err(FetchFailure::Failed(cause)) => (self.shared.failed(cause), Err(cause)),
            Ok(fetched) => {
                let rows = match self.persistence {
                    Persistence::Off => Vec::new(),
                    Persistence::File(_) => fetched.rows().to_vec(),
                };
                let judged = self.publish(fetched, deadline);
                if judged.1.is_ok() {
                    self.offer(&rows);
                }
                judged
            }
        };
        report(previous, result);
        // Whatever this run read, a publication still waiting is retried.
        self.persist(deadline).await;
        result.map_err(LoadFailed)
    }

    /// Queues the published control-plane snapshot for the last-good file.
    fn offer(&self, rows: &[CandidateRow]) {
        let Persistence::File(persister) = &self.persistence else {
            return;
        };
        let Some(record) = self.shared.reader().record() else {
            return;
        };
        if let Some(file) = last_good::file_of(self.shared.reader().base(), record.snapshot(), rows)
        {
            persister.offer(file);
        }
    }

    /// Writes what waits for the last-good file. A failure is recorded and
    /// never changes the run's outcome.
    async fn persist(&self, deadline: Instant) {
        let Persistence::File(persister) = &self.persistence else {
            return;
        };
        let by = (Instant::now() + FILE_SUBDEADLINE).min(work_deadline(deadline));
        if let Err(failure) = persister.attempt(by).await {
            self.shared.persist_failed(failure);
        }
    }

    /// Judges one read and publishes it if it may be published: a version
    /// below the applied one is a regression whatever its rows hold, then
    /// the rows are validated, then the writer decides. Nothing is published
    /// once the run's deadline has passed, however long validation took.
    fn publish(&self, fetched: Fetched, deadline: Instant) -> (LastRun, Result<(), LoadCause>) {
        let observed = fetched.version;
        let mut writer = self.writer.lock().unwrap_or_else(PoisonError::into_inner);
        let current = writer.published();
        if let Some(record) = &current
            && let Standing::ControlPlane {
                version: applied, ..
            } = record.authority().standing()
            && observed < applied
        {
            let (previous, cause) = self
                .shared
                .refused(Refusal::Regressed { applied, observed }, Some(observed));
            return (previous, Err(cause));
        }
        let mut candidate = fetched.candidate(&self.scope);
        if let Some(record) = &current {
            // Carrying keeps a held, engaged switch engaged. The writer only
            // ever publishes this base's snapshots, so carrying cannot fail.
            match candidate.carrying(record.snapshot()) {
                Ok(carried) => candidate = carried,
                Err(_) => {
                    return (
                        self.shared.failed(LoadCause::InvalidData),
                        Err(LoadCause::InvalidData),
                    );
                }
            }
        }
        let validated = Snapshot::validate(&self.scope, candidate);
        if Instant::now() >= deadline {
            return (
                self.shared.failed(LoadCause::Deadline),
                Err(LoadCause::Deadline),
            );
        }
        let disposition = match validated {
            Err(rejection) => writer.reject(rejection),
            Ok(snapshot) => writer.apply(Arc::new(snapshot), Instant::now()),
        };
        match disposition {
            Disposition::Applied | Disposition::Confirmed => (self.shared.loaded(), Ok(())),
            Disposition::Refused(refusal) => {
                let (previous, cause) = self.shared.refused(refusal, Some(observed));
                (previous, Err(cause))
            }
        }
    }
}

/// How loud a run's `flags.load` event is: WARN when a failure's cause differs
/// from the previous run's, INFO when a run recovers, DEBUG otherwise, so a
/// hundred failing polls with one cause are one warning.
pub(crate) fn level(previous: LastRun, result: Result<(), LoadCause>) -> tracing::Level {
    match (result, previous) {
        (Ok(()), LastRun::Failed(_)) => tracing::Level::INFO,
        (Ok(()), LastRun::None | LastRun::Loaded) => tracing::Level::DEBUG,
        (Err(cause), LastRun::Failed(before)) if before == cause => tracing::Level::DEBUG,
        (Err(_), LastRun::None | LastRun::Loaded | LastRun::Failed(_)) => tracing::Level::WARN,
    }
}

/// Emits the run's `flags.load` event at [`level`].
fn report(previous: LastRun, result: Result<(), LoadCause>) {
    let (outcome, cause) = match result {
        Ok(()) => ("loaded", ""),
        Err(cause) => ("failed", cause.as_str()),
    };
    let level = level(previous, result);
    #[cfg(test)]
    EMITTED.with(|emitted| emitted.borrow_mut().push((level, cause)));
    match level {
        tracing::Level::WARN => warn(outcome, cause),
        tracing::Level::INFO => info(outcome, cause),
        _ => debug(outcome, cause),
    }
}

#[cfg(test)]
thread_local! {
    /// Every load event this thread's runs emitted, with its cause.
    pub(crate) static EMITTED: std::cell::RefCell<Vec<(tracing::Level, &'static str)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

macro_rules! load_event {
    ($name:ident, $level:expr) => {
        fn $name(outcome: &'static str, cause: &'static str) {
            tracing::event!(
                name: "flags.load",
                target: "batter::flags",
                $level,
                batter.flags.load.outcome = outcome,
                batter.flags.load.cause = cause,
                "flags load",
            );
        }
    };
}

load_event!(warn, tracing::Level::WARN);
load_event!(info, tracing::Level::INFO);
load_event!(debug, tracing::Level::DEBUG);
