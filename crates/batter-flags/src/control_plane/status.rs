//! What a registered loader reports: its status, in precedence order, and the
//! readiness condition built on it.

use std::{
    sync::{Arc, Mutex, PoisonError},
    time::Duration,
};

use batter_core::readiness::ReadinessCondition;
use batter_flags_core::{
    Version,
    low_level::{Refusal, SnapshotRejection, Standing},
};
use tokio::time::Instant;

use crate::reader::{Publication, Reader};

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

/// Why the latest read at or above the applied version could not publish.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RejectionCause {
    /// The applied version came back with a different digest.
    DigestDrift,
    /// The candidate is structurally invalid.
    Structural(SnapshotRejection),
}

/// The state of a registered loader, in precedence order: the first arm that
/// applies is the one reported.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SnapshotStatus {
    /// The loader has ended; nothing will be published again.
    LoaderStopped,
    /// The installed schema is older than this build requires. Nothing is
    /// read or published until it is migrated, publication or not.
    SchemaBehind {
        /// The revision this build requires.
        required: i32,
        /// The installed revision, 0 when no flags migration has run.
        found: i32,
    },
    /// The latest read is below the applied control-plane version.
    Regressed {
        /// The applied version, which stays served.
        applied: Version,
        /// The version the latest read returned.
        observed: Version,
    },
    /// The latest read could not be published; what was served stays.
    Rejected {
        /// The version the read carried, when it carried one.
        observed: Option<Version>,
        /// Why it was refused.
        cause: RejectionCause,
    },
    /// Nothing is published; views serve the declared base.
    NotLoaded {
        /// How long ago the flags were registered.
        since_registration: Duration,
    },
    /// The last-good file is served and no control-plane read is yet.
    ServingPersisted {
        /// The file's version.
        version: Version,
    },
    /// A control-plane publication is served but its confirmation is older
    /// than the maximum age, so views are captured Stale.
    Stale,
    /// A control-plane publication is served and recently confirmed.
    Fresh,
}

/// What a readiness condition requires of the flags.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Gate {
    /// Unready while the loader has stopped, the schema is behind, or nothing
    /// (neither a control-plane read nor a last-good file) is published.
    Published,
    /// Unready unless the status is [`SnapshotStatus::Fresh`].
    Fresh,
}

/// The name of the flags readiness condition, for either gate.
pub(crate) const CONDITION: &str = "flags.snapshot";

/// Whether the installed schema was found current on the latest read that
/// reached it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Schema {
    /// No read has reached the schema, or the last one found it current.
    Current,
    /// The last read found it behind.
    Behind { found: i32 },
}

/// Whether the latest read at or above the applied version was refused.
/// A refusal clears only through a publication or a confirming read.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Verdict {
    Clear,
    Refused {
        refusal: Refusal,
        observed: Option<Version>,
    },
}

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
    schema: Schema,
    verdict: Verdict,
    last_run: LastRun,
    last_rejection: Option<Refusal>,
}

/// What the loader shares with the registered flags.
pub(crate) struct Shared {
    reader: Arc<Reader>,
    state: Mutex<State>,
}

impl Shared {
    pub(crate) fn new(reader: Arc<Reader>) -> Self {
        Self {
            reader,
            state: Mutex::new(State {
                schema: Schema::Current,
                verdict: Verdict::Clear,
                last_run: LastRun::None,
                last_rejection: None,
            }),
        }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Records a run that published or confirmed, and returns the cause of
    /// the run before it.
    pub(crate) fn loaded(&self) -> LastRun {
        let mut state = self.state();
        state.schema = Schema::Current;
        state.verdict = Verdict::Clear;
        std::mem::replace(&mut state.last_run, LastRun::Loaded)
    }

    /// Records a read that found the schema behind.
    pub(crate) fn schema_behind(&self, found: i32) -> LastRun {
        let mut state = self.state();
        state.schema = Schema::Behind { found };
        std::mem::replace(
            &mut state.last_run,
            LastRun::Failed(LoadCause::SchemaBehind),
        )
    }

    /// Records a read that failed before it could be judged.
    pub(crate) fn failed(&self, cause: LoadCause) -> LastRun {
        std::mem::replace(&mut self.state().last_run, LastRun::Failed(cause))
    }

    /// Records a read the writer refused, and returns the previous run's
    /// cause and this one's.
    pub(crate) fn refused(
        &self,
        refusal: Refusal,
        observed: Option<Version>,
    ) -> (LastRun, LoadCause) {
        let mut state = self.state();
        let cause = cause(refusal);
        state.schema = Schema::Current;
        state.verdict = Verdict::Refused { refusal, observed };
        state.last_rejection = Some(refusal);
        (
            std::mem::replace(&mut state.last_run, LastRun::Failed(cause)),
            cause,
        )
    }

    pub(crate) fn schema_is_behind(&self) -> bool {
        matches!(self.state().schema, Schema::Behind { .. })
    }

    pub(crate) fn last_rejection(&self) -> Option<Refusal> {
        self.state().last_rejection
    }

    pub(crate) fn reader(&self) -> &Reader {
        &self.reader
    }

    pub(crate) fn status(&self, now: Instant) -> SnapshotStatus {
        if self.reader.writer_gone() {
            return SnapshotStatus::LoaderStopped;
        }
        let (schema, verdict) = {
            let state = self.state();
            (state.schema, state.verdict)
        };
        if let Schema::Behind { found } = schema {
            return SnapshotStatus::SchemaBehind {
                required: batter_flags_postgres::REQUIRED_SCHEMA_REVISION,
                found,
            };
        }
        if let Verdict::Refused { refusal, observed } = verdict {
            match refusal {
                Refusal::Regressed { applied, observed } => {
                    return SnapshotStatus::Regressed { applied, observed };
                }
                Refusal::DigestDrift { .. } => {
                    return SnapshotStatus::Rejected {
                        observed,
                        cause: RejectionCause::DigestDrift,
                    };
                }
                Refusal::Rejected(rejection) => {
                    return SnapshotStatus::Rejected {
                        observed,
                        cause: RejectionCause::Structural(rejection),
                    };
                }
                // The loader reads only the control plane, so a file can never
                // follow a publication here.
                Refusal::FileAfterPublication => {}
            }
        }
        match self.reader.publication(now) {
            Publication::Unpublished { since_registration } => {
                SnapshotStatus::NotLoaded { since_registration }
            }
            Publication::Published { standing, fresh } => match standing {
                Standing::Unpublished => SnapshotStatus::NotLoaded {
                    since_registration: Duration::ZERO,
                },
                Standing::Persisted { version } => SnapshotStatus::ServingPersisted { version },
                Standing::ControlPlane { .. } if fresh => SnapshotStatus::Fresh,
                Standing::ControlPlane { .. } => SnapshotStatus::Stale,
            },
        }
    }

    /// Whether `gate` is satisfied now.
    pub(crate) fn satisfies(&self, gate: Gate) -> bool {
        let status = self.status(Instant::now());
        match gate {
            Gate::Fresh => status == SnapshotStatus::Fresh,
            Gate::Published => match status {
                SnapshotStatus::LoaderStopped
                | SnapshotStatus::SchemaBehind { .. }
                | SnapshotStatus::NotLoaded { .. } => false,
                SnapshotStatus::Regressed { .. } | SnapshotStatus::Rejected { .. } => {
                    self.reader.has_publication()
                }
                SnapshotStatus::ServingPersisted { .. }
                | SnapshotStatus::Stale
                | SnapshotStatus::Fresh => true,
            },
        }
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
