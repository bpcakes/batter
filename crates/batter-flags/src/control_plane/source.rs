//! The loader's one seam: where a refresh reads its candidate from.
//!
//! Production reads the `PostgreSQL` store. Tests drive versions, rows,
//! failures and schema revisions through a fake. The seam is crate-private and
//! not an extension point.

use batter_flags_core::{
    Version,
    low_level::{Candidate, CandidateRow, ValidationScope},
};
use batter_flags_postgres::{ErrorClass, Loaded, ServingStore, StoreError};
use tokio::time::Instant;

use super::LoadCause;

/// Where a refresh reads from.
pub(crate) enum LoadSource {
    /// The audited store.
    Store(ServingStore),
    /// A scripted source for paused-time tests.
    #[cfg(test)]
    Fake(std::sync::Arc<fake::Fake>),
}

/// One read: the store's version and its rows, before validation. The
/// store's read is kept whole and consumed by candidate conversion, so a poll
/// copies its rows only when persistence asks for them.
pub(crate) struct Fetched(Read);

enum Read {
    Store(Loaded),
    #[cfg(test)]
    Scripted {
        version: Version,
        rows: Vec<CandidateRow>,
    },
}

#[derive(Clone, Copy)]
pub(crate) enum Observed {
    Store(batter_flags_postgres::Publication),
    #[cfg(test)]
    Scripted(Version),
}

impl Observed {
    pub(crate) fn version(self) -> Version {
        match self {
            Self::Store(publication) => publication.version(),
            #[cfg(test)]
            Self::Scripted(version) => version,
        }
    }
}

impl Fetched {
    pub(crate) fn observed(&self) -> Observed {
        match &self.0 {
            Read::Store(loaded) => Observed::Store(loaded.publication()),
            #[cfg(test)]
            Read::Scripted { version, .. } => Observed::Scripted(*version),
        }
    }

    pub(crate) fn rows(&self) -> &[CandidateRow] {
        match &self.0 {
            Read::Store(loaded) => loaded.rows(),
            #[cfg(test)]
            Read::Scripted { rows, .. } => rows,
        }
    }

    pub(crate) fn candidate(self, scope: &ValidationScope) -> Candidate {
        match self.0 {
            Read::Store(loaded) => loaded.candidate(scope),
            #[cfg(test)]
            Read::Scripted { version, rows } => Candidate::new(scope, version).rows(rows),
        }
    }

    #[cfg(test)]
    pub(crate) fn scripted(version: Version, rows: Vec<CandidateRow>) -> Self {
        Self(Read::Scripted { version, rows })
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
                Ok(loaded) => Ok(Fetched(Read::Store(loaded))),
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
                Script::Serve { version, rows } => Ok(Fetched::scripted(version, rows)),
                Script::Fail(failure) => Err(failure),
                Script::Hang => {
                    tokio::time::sleep_until(deadline).await;
                    Err(FetchFailure::Failed(super::LoadCause::Deadline))
                }
            }
        }
    }
}
