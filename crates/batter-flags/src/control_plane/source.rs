//! The loader's one seam: where a refresh reads its candidate from.
//!
//! Production reads the `PostgreSQL` store. Tests drive versions, rows,
//! failures and schema revisions through a fake. The seam is crate-private and
//! not an extension point.

use batter_flags_core::{
    Version,
    low_level::{Candidate, CandidateRow, ValidationScope},
};
use batter_flags_postgres::{ErrorClass, PooledStore, StoreError};
use tokio::time::Instant;

use super::LoadCause;

/// Where a refresh reads from.
pub(crate) enum LoadSource {
    /// The audited store.
    Store(PooledStore),
    /// A scripted source for paused-time tests.
    #[cfg(test)]
    Fake(std::sync::Arc<fake::Fake>),
}

/// One read: the store's version and its rows, before validation.
pub(crate) struct Fetched {
    pub(crate) version: Version,
    rows: Vec<CandidateRow>,
}

impl Fetched {
    pub(crate) fn rows(&self) -> &[CandidateRow] {
        &self.rows
    }

    pub(crate) fn candidate(self, scope: &ValidationScope) -> Candidate {
        Candidate::new(scope).at(self.version).rows(self.rows)
    }
}

/// Why a read returned no candidate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FetchFailure {
    /// The installed schema is older than this build requires.
    SchemaBehind { found: i32 },
    /// Any other failure, by cause.
    Failed(LoadCause),
}

impl LoadSource {
    /// Reads once, within `deadline`. Nothing here retries.
    pub(crate) async fn fetch(&self, deadline: Instant) -> Result<Fetched, FetchFailure> {
        match self {
            Self::Store(store) => match store.load(deadline).await {
                Ok(loaded) => Ok(Fetched {
                    version: loaded.version(),
                    rows: loaded.rows().to_vec(),
                }),
                Err(error) => Err(classify(&error)),
            },
            #[cfg(test)]
            Self::Fake(fake) => fake.fetch(deadline).await,
        }
    }
}

fn classify(error: &StoreError) -> FetchFailure {
    // A schema observation outranks whatever went wrong after it, such as a
    // rollback that failed or ran out of time.
    if let Some(found) = error.installed_schema_revision() {
        return FetchFailure::SchemaBehind { found };
    }
    match error.class() {
        ErrorClass::SchemaRevision => FetchFailure::SchemaBehind { found: 0 },
        ErrorClass::Standby => FetchFailure::Failed(LoadCause::Standby),
        ErrorClass::Deadline => FetchFailure::Failed(LoadCause::Deadline),
        ErrorClass::InvalidData => FetchFailure::Failed(LoadCause::InvalidData),
        // Only a migration refuses for its search path; a load never does.
        ErrorClass::UnsupportedServer
        | ErrorClass::Database
        | ErrorClass::CommitUnknown
        | ErrorClass::Interrupted
        | ErrorClass::SearchPath => FetchFailure::Failed(LoadCause::Unavailable),
    }
}

#[cfg(test)]
pub(crate) mod fake {
    //! A scripted store: what the next reads return, and how many there were.

    use std::sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    };

    use batter_flags_core::{Version, low_level::CandidateRow};
    use tokio::time::Instant;

    use super::{FetchFailure, Fetched};

    /// What every read returns until the script changes.
    #[derive(Clone)]
    pub(crate) enum Script {
        Serve {
            version: Version,
            rows: Vec<CandidateRow>,
        },
        Fail(FetchFailure),
        /// The read never returns; its deadline ends it.
        Hang,
    }

    pub(crate) struct Fake {
        script: Mutex<Script>,
        reads: AtomicUsize,
    }

    impl std::fmt::Debug for Fake {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("Fake")
                .field("reads", &self.reads())
                .finish_non_exhaustive()
        }
    }

    impl Fake {
        pub(crate) fn new(script: Script) -> Self {
            Self {
                script: Mutex::new(script),
                reads: AtomicUsize::new(0),
            }
        }

        pub(crate) fn set(&self, script: Script) {
            *self
                .script
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = script;
        }

        pub(crate) fn reads(&self) -> usize {
            self.reads.load(Ordering::SeqCst)
        }

        pub(crate) async fn fetch(&self, deadline: Instant) -> Result<Fetched, FetchFailure> {
            self.reads.fetch_add(1, Ordering::SeqCst);
            let script = self
                .script
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone();
            match script {
                Script::Serve { version, rows } => Ok(Fetched { version, rows }),
                Script::Fail(failure) => Err(failure),
                Script::Hang => {
                    tokio::time::sleep_until(deadline).await;
                    Err(FetchFailure::Failed(super::LoadCause::Deadline))
                }
            }
        }
    }
}
