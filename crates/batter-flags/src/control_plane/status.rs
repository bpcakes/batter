//! What a registered loader reports: its status, as separate loader, served
//! and latest-read facts, and the readiness condition built on it.

use std::{
    sync::{Arc, Mutex, PoisonError},
    time::Duration,
};

use batter_core::readiness::ReadinessCondition;
use batter_flags_core::{
    Freshness, Version,
    low_level::{Candidate, Disposition, Provenance, Record, Refusal},
};
use tokio::time::Instant;

use crate::reader::{Late, LiveRecord, Reader, Writer};

/// Why a refresh published nothing.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum LoadCause {
    /// The installed schema is older than this build requires.
    SchemaBehind,
    /// The pool points at a standby, whose reads cannot renew freshness.
    Standby,
    /// The run's deadline passed before the read returned.
    Deadline,
    /// The database could not be reached or failed the read.
    Unavailable,
    /// A stored row could not be decoded at all.
    InvalidData,
    /// The read is below the applied control-plane version.
    Regressed,
    /// The read repeats the applied version with other content.
    DigestDrift,
    /// The candidate is structurally invalid.
    Rejected,
}

impl LoadCause {
    /// The cause as the `flags.load` event names it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SchemaBehind => "schema_behind",
            Self::Standby => "standby",
            Self::Deadline => "deadline",
            Self::Unavailable => "unavailable",
            Self::InvalidData => "invalid_data",
            Self::Regressed => "regressed",
            Self::DigestDrift => "digest_drift",
            Self::Rejected => "rejected",
        }
    }
}

/// What a registered loader reports: whether it still runs, what views serve
/// and what the latest judged read did.
///
/// The three are separate facts, read together in one critical section with
/// the publication they describe, so a refusal never hides what is served and
/// a status never pairs one publication with another read's verdict. A
/// [`Gate`] is judged from one status with [`satisfies`](Self::satisfies).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SnapshotStatus {
    /// Whether the refresh still runs.
    pub loader: LoaderState,
    /// What views captured now serve.
    pub served: Served,
    /// What the latest read that reached the store and was judged did.
    pub latest_read: LatestRead,
}

/// Whether a registered loader can still publish.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LoaderState {
    /// The refresh component runs.
    Running,
    /// The refresh has ended; nothing will be published again, and what was
    /// last published keeps serving.
    Stopped,
}

/// What views captured now serve.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Served {
    /// Nothing is published; views serve the declared base.
    DeclaredBase {
        /// How long ago the flags were registered.
        since_registration: Duration,
    },
    /// The last-good file restored at startup; no control-plane read has
    /// applied yet. A file carries no clock, so its views age from
    /// registration.
    PersistedFile {
        /// The file's version.
        version: Version,
    },
    /// A control-plane publication.
    ControlPlane {
        /// The applied version.
        version: Version,
        /// Whether its last confirmation is within the maximum age; views are
        /// captured with this freshness.
        freshness: Freshness,
    },
}

/// What the latest read that reached the store and was judged did. A read
/// that failed before it could be judged, such as a timeout, changes nothing
/// here.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LatestRead {
    /// No read has been judged since registration.
    NoRead,
    /// It published a snapshot or confirmed the one served.
    Loaded,
    /// It found the installed schema older than this build requires. Nothing
    /// is read or published until it is migrated.
    SchemaBehind {
        /// The revision this build requires.
        required: i32,
        /// The installed revision, 0 when no flags migration has run.
        found: i32,
    },
    /// The publisher refused it; what is served stays. A refusal clears only
    /// through a publication or a confirming read.
    Refused {
        /// Why it was refused.
        refusal: Refusal,
        /// The version the store returned.
        observed: Version,
    },
}

/// What a readiness condition requires of the flags.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Gate {
    /// Unready while the loader has stopped, the latest read found the schema
    /// behind, or nothing (neither a control-plane read nor a last-good file)
    /// is published.
    Published,
    /// Unready unless the loader runs, the latest judged read loaded, and a
    /// control-plane publication is served fresh.
    Fresh,
}

impl SnapshotStatus {
    /// Whether `gate` is satisfied by this status alone.
    pub fn satisfies(&self, gate: Gate) -> bool {
        let running = match self.loader {
            LoaderState::Running => true,
            LoaderState::Stopped => false,
        };
        match gate {
            Gate::Published => {
                running
                    && !matches!(self.latest_read, LatestRead::SchemaBehind { .. })
                    && match self.served {
                        Served::DeclaredBase { .. } => false,
                        Served::PersistedFile { .. } | Served::ControlPlane { .. } => true,
                    }
            }
            Gate::Fresh => {
                running
                    && self.latest_read == LatestRead::Loaded
                    && matches!(
                        self.served,
                        Served::ControlPlane {
                            freshness: Freshness::Fresh,
                            ..
                        }
                    )
            }
        }
    }
}

/// The name of the flags readiness condition, for either gate.
pub(crate) const CONDITION: &str = "flags.snapshot";

/// The cause of the latest run, for event deduplication.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LastRun {
    /// No run has finished.
    None,
    /// The latest run published or confirmed.
    Loaded,
    /// The latest run failed for this cause.
    Failed(LoadCause),
}

struct State {
    latest_read: LatestRead,
    last_run: LastRun,
    last_rejection: Option<Refusal>,
}

/// What the loader shares with the registered flags.
///
/// Its lock is held while the refresh publishes, so a status read under it
/// pairs a publication with that same read's verdict.
pub(crate) struct Shared {
    reader: Arc<Reader>,
    live: LiveRecord,
    state: Mutex<State>,
}

impl Shared {
    pub(crate) fn new(reader: Arc<Reader>, live: LiveRecord) -> Self {
        Self {
            reader,
            live,
            state: Mutex::new(State {
                latest_read: LatestRead::NoRead,
                last_run: LastRun::None,
                last_rejection: None,
            }),
        }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Publishes one read read at `observed` through `writer` and records
    /// what it did, in one critical section of the shared state, and returns
    /// the cause of the run before it and this run's outcome.
    pub(crate) fn publish(
        &self,
        writer: &mut Writer,
        candidate: Candidate,
        deadline: Instant,
        observed: Version,
    ) -> (LastRun, Result<(), LoadCause>) {
        let mut state = self.state();
        let (latest_read, result) = match writer.read(candidate, deadline) {
            Err(Late) => (state.latest_read, Err(LoadCause::Deadline)),
            Ok(Disposition::Applied | Disposition::Confirmed) => (LatestRead::Loaded, Ok(())),
            Ok(Disposition::Refused(refusal)) => {
                state.last_rejection = Some(refusal);
                (
                    LatestRead::Refused { refusal, observed },
                    Err(cause(refusal)),
                )
            }
        };
        state.latest_read = latest_read;
        let run = match result {
            Ok(()) => LastRun::Loaded,
            Err(cause) => LastRun::Failed(cause),
        };
        (std::mem::replace(&mut state.last_run, run), result)
    }

    /// Records a read that found the schema behind.
    pub(crate) fn schema_behind(&self, found: i32) -> LastRun {
        let mut state = self.state();
        state.latest_read = LatestRead::SchemaBehind {
            required: batter_flags_postgres::REQUIRED_SCHEMA_REVISION,
            found,
        };
        std::mem::replace(
            &mut state.last_run,
            LastRun::Failed(LoadCause::SchemaBehind),
        )
    }

    /// Records a read that failed before it could be judged.
    pub(crate) fn failed(&self, cause: LoadCause) -> LastRun {
        std::mem::replace(&mut self.state().last_run, LastRun::Failed(cause))
    }

    pub(crate) fn schema_is_behind(&self) -> bool {
        matches!(self.state().latest_read, LatestRead::SchemaBehind { .. })
    }

    pub(crate) fn last_rejection(&self) -> Option<Refusal> {
        self.state().last_rejection
    }

    pub(crate) fn reader(&self) -> &Reader {
        &self.reader
    }

    /// The control plane's side of the reader.
    pub(crate) fn live(&self) -> &LiveRecord {
        &self.live
    }

    pub(crate) fn status(&self, now: Instant) -> SnapshotStatus {
        // The refresh publishes under this lock, so the record and the
        // verdict read here belong to the same read.
        let state = self.state();
        let loader = if self.live.writer_gone() {
            LoaderState::Stopped
        } else {
            LoaderState::Running
        };
        let record = self.live.current();
        let served = match &*record {
            Record::Unpublished(unpublished) => Served::DeclaredBase {
                since_registration: now.saturating_duration_since(*unpublished.registered()),
            },
            Record::Published(published) => {
                let version = published.snapshot().version();
                match published.snapshot().provenance() {
                    Provenance::LastGoodFile => Served::PersistedFile { version },
                    Provenance::ControlPlane => Served::ControlPlane {
                        version,
                        freshness: self.live.freshness(&record, now),
                    },
                }
            }
        };
        SnapshotStatus {
            loader,
            served,
            latest_read: state.latest_read,
        }
    }

    /// Whether `gate` is satisfied now, judged from one status.
    pub(crate) fn satisfies(&self, gate: Gate) -> bool {
        self.status(Instant::now()).satisfies(gate)
    }

    pub(crate) fn condition() -> ReadinessCondition {
        ReadinessCondition::new(CONDITION).expect("the flags condition name is valid")
    }
}

fn cause(refusal: Refusal) -> LoadCause {
    match refusal {
        Refusal::Regressed { .. } => LoadCause::Regressed,
        Refusal::DigestDrift { .. } => LoadCause::DigestDrift,
        Refusal::Rejected(_) | Refusal::FileAfterPublication => LoadCause::Rejected,
    }
}

#[cfg(test)]
mod tests;
