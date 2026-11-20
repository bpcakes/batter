use crate::{Deferral, UnmetInvariant};
use batter_flags_core::Version;
use batter_sqlx::CommitUnconfirmed;
use std::{error::Error, fmt};

/// A declarer's server-clock presence at validation time.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Presence {
    /// Heartbeat has not expired and the process has not stopped.
    Live,
    /// Heartbeat expired, but the process has not stopped or been pruned.
    Silent,
}
/// A blocking build; text is available explicitly, never through Debug.
#[derive(Clone, Eq, PartialEq)]
pub struct BlockingBuild {
    /// Application-provided build label.
    pub build: String,
    /// Server-clock presence class.
    pub presence: Presence,
}
impl fmt::Debug for BlockingBuild {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BlockingBuild")
            .field("presence", &self.presence)
            .finish_non_exhaustive()
    }
}
/// Why no live process declares a successful mutation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UndeclaredBasis {
    /// A Silent or Stopped process references a declaring catalog.
    NoLiveDeclarer,
    /// Only unreferenced retained catalogs declare the key.
    Orphaned,
    /// An explicit request supplied the first declaration.
    NeverDeclared,
}
/// Certain, non-mutating policy refusal. No arm authorizes replay.
#[derive(Debug, Eq, PartialEq)]
pub enum Refusal {
    /// The stored state cannot be decoded by this writer.
    UnknownRowEncoding,
    /// A switch/flag or explicit type mismatch.
    WrongKind {
        /// Stored kind/type tag.
        stored: String,
        /// Requested kind/type tag.
        requested: String,
    },
    /// The key is a toggle, which takes only `on` or `off`, and the
    /// requested variant is neither. Judged from the stored row or retained
    /// declaration alone, so no Live or Silent declarer is needed to refuse it.
    NotAToggleVariant,
    /// Ordinary writes cannot change an archived key.
    Archived,
    /// Archive requires all declarers to be stopped or pruned.
    StillDeclared {
        /// Live and Silent blocking builds.
        builds: Vec<BlockingBuild>,
    },
    /// A retained declaration is restricted to declared mode.
    EnvironmentRestricted,
    /// No retained catalog or stored row supplies the requested key.
    Undeclared,
    /// A Live or Silent declaration cannot decode the requested state.
    UndecodableByLiveDeclaration {
        /// Blocking builds, with presence classifications.
        builds: Vec<BlockingBuild>,
    },
    /// The caller's CAS revision exceeds the database head.
    RevisionAboveHead {
        /// The key's current revision, 0 for an absent row.
        current: Version,
        /// Actual database head.
        head: Version,
    },
    /// A retained catalog declares the flip coordinated, and the request
    /// refused coordinated flips: processes apply a change poll by poll,
    /// never all at once.
    CoordinatedFlip,
    /// The written flag would newly hold flags in a Live or Silent
    /// declarer's processes, for cross-flag invariants it leaves unmet, and
    /// the request refused that.
    InvariantUnmet {
        /// The flags it would newly hold, sorted; never empty.
        unmet: Vec<UnmetInvariant>,
    },
    /// A Live or Silent declarer's catalog could not be judged, because
    /// this writer cannot decode its manifest or because it holds more than
    /// four governing switches, and the request refused that.
    InvariantsUnjudged {
        /// The catalogs that could not be judged, by cause.
        catalogs: crate::UnjudgedCatalogs,
    },
}
/// Commit acknowledgement was not obtained. Reconcile; never blindly replay.
/// Native completion failures retain `batter-sqlx`'s original evidence.
pub enum CommitUncertainty {
    /// Native commit completion failure.
    Native(CommitUnconfirmed),
    /// Budget expired while the consuming commit future was active.
    /// The upstream owner does not expose its send point; this is conservative.
    Deadline,
}
impl fmt::Debug for CommitUncertainty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
impl fmt::Display for CommitUncertainty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("flags commit outcome unknown")
    }
}
impl Error for CommitUncertainty {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Native(e) => Some(e),
            Self::Deadline => None,
        }
    }
}
/// An audit event identified across restores. Its text form travels from the
/// acknowledged write report to the operator's rollout query.
///
/// ```
/// use batter_flags_postgres::WriteId;
/// let text = "8/6d76ed65-80cf-4ec2-a203-de11dfa82e89";
/// let write: WriteId = text.parse().unwrap();
/// assert_eq!(write.version().get(), 8);
/// assert_eq!(write.to_string(), text);
/// ```
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WriteId {
    pub(crate) version: Version,
    pub(crate) event: uuid::Uuid,
}

impl WriteId {
    /// Counter allocated to this audit event.
    pub fn version(self) -> Version {
        self.version
    }
}

impl std::fmt::Display for WriteId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.version.get(), self.event)
    }
}

impl std::str::FromStr for WriteId {
    type Err = crate::ValidationError;
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let (version, event) = text.split_once('/').ok_or(crate::ValidationError)?;
        let version: i64 = version.parse().map_err(|_| crate::ValidationError)?;
        if version <= 0 {
            return Err(crate::ValidationError);
        }
        Ok(Self {
            version: Version::new(version as u64),
            event: event.parse().map_err(|_| crate::ValidationError)?,
        })
    }
}
/// Exhaustive write/archive/restore result, released only after disposition.
#[derive(Debug)]
#[must_use]
pub enum WriteOutcome {
    /// Acknowledged write with at least one live declaring process.
    Written {
        /// Identity to pass to [`crate::PooledStore::rollout`].
        /// Migration: read the former `version` field with `write_id.version()`.
        write_id: WriteId,
        /// Previous state, available for deliberate inspection.
        previous: Option<StoredState>,
        /// Number of live declaring processes.
        live_declaring: usize,
        /// Why processes will not serve the state as written for now.
        deferral: Deferral,
    },
    /// Acknowledged write without a live declaring process.
    WrittenUndeclared {
        /// Identity to pass to [`crate::PooledStore::rollout`].
        /// Migration: read the former `version` field with `write_id.version()`.
        write_id: WriteId,
        /// Why there was no live declaration.
        basis: UndeclaredBasis,
        /// Why a Silent declarer's processes will not serve the state as
        /// written, should they still run.
        deferral: Deferral,
    },
    /// Identical state; no counter or audit change.
    Unchanged {
        /// The row's last update (zero when absent).
        version: Version,
    },
    /// The row changed after the requested CAS revision.
    RevisionMismatch {
        /// Row's last update.
        current: Version,
        /// Store head observed under the counter lock.
        head: Version,
    },
    /// Certain, rolled-back policy refusal.
    Refused(Refusal),
    /// The proposed write may have committed. Its version is provisional.
    Uncertain {
        /// Proposed counter, for explicit reconciliation.
        proposed_version: Version,
        /// Native failure or commit-phase deadline.
        cause: CommitUncertainty,
    },
}
/// Retained row state. Formatting never prints the JSON value.
#[derive(Clone, PartialEq)]
pub struct StoredState(pub(crate) serde_json::Value);
impl StoredState {
    /// Explicitly inspect the stored encoding and value.
    pub fn value(&self) -> &serde_json::Value {
        &self.0
    }
}
impl fmt::Debug for StoredState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("StoredState([redacted])")
    }
}
