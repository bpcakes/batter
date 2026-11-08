//! The last-good file: the control plane's last publication, kept on disk so
//! a restart during an outage serves it instead of the bare declared base.
//!
//! The file is a cache. Deleting it is always safe; a missing, damaged or
//! foreign file publishes nothing and never fails startup.

pub(crate) mod format;
mod path;
mod persist;

use std::{collections::HashSet, sync::Arc};

use batter_flags_core::{
    DeclarationFingerprint, FlagKey, Version,
    low_level::{Base, Candidate, CandidateRow, Provenance, Snapshot, SnapshotRejection},
};

use self::format::EntryValue;
pub(crate) use self::path::Initial;
pub use self::path::{SnapshotDirectory, SnapshotFilePath};
pub(crate) use self::persist::Persister;

/// Why a snapshot directory cannot be used. Startup fails on these.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum DirectoryError {
    /// The directory path is relative.
    #[error("the last-good directory must be an absolute path")]
    NotAbsolute,
    /// The file name is outside its grammar.
    #[error("the last-good file name must be 1 to 64 ASCII letters, digits, `.`, `_` or `-`")]
    InvalidName,
    /// The directory does not exist or cannot be read.
    #[error("the last-good directory does not exist or cannot be read")]
    Missing,
    /// The path is not a directory.
    #[error("the last-good path is not a directory")]
    NotADirectory,
    /// This process cannot create and remove a file in the directory.
    #[error("the last-good directory is not writable by this process")]
    NotWritable,
}

/// Why the file found at startup published nothing.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum FileProblem {
    /// The file could not be read.
    #[error("the last-good file could not be read")]
    Unreadable,
    /// The file is larger than 1 MiB.
    #[error("the last-good file is larger than its limit")]
    TooLarge,
    /// The file ends before its structure does.
    #[error("the last-good file is truncated")]
    Truncated,
    /// Bytes follow the trailer.
    #[error("the last-good file has trailing bytes")]
    TrailingBytes,
    /// The trailer does not match the content.
    #[error("the last-good file does not match its checksum")]
    Tampered,
    /// The magic or a field is malformed.
    #[error("the last-good file is malformed")]
    Malformed,
    /// The file uses a format version this build does not read.
    #[error("the last-good file uses an unsupported format version")]
    UnsupportedVersion,
    /// The file was written for another settings prefix or process kind.
    #[error("the last-good file was written for another process scope")]
    Foreign,
    /// The entries are structurally invalid.
    #[error("the last-good file's entries are invalid")]
    Rejected(SnapshotRejection),
}

/// Why a persist did not write. It never changes a run's outcome.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum PersistFailure {
    /// Another persist, in this process or another, holds the file.
    #[error("another writer holds the last-good file")]
    Busy,
    /// The persist did not finish within its sub-deadline.
    #[error("the last-good persist ran out of time")]
    Deadline,
    /// The file system refused a step before the rename; the file on disk
    /// is unchanged.
    #[error("the last-good persist failed")]
    Io,
    /// The file was replaced, but syncing its directory failed, so the new
    /// file may not survive a power loss. The write is retried.
    #[error("the last-good file was replaced but its durability is unconfirmed")]
    NotDurable,
    /// The snapshot does not fit the file's limit.
    #[error("the snapshot is too large for the last-good file")]
    TooLarge,
}

/// What the last-good file did at startup.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StartupFile {
    /// No directory was configured.
    Disabled,
    /// The directory holds no file: nothing was published from it.
    Missing,
    /// The file was published before the driver started.
    Published {
        /// The file's version.
        version: Version,
    },
    /// A file was found but could not be used, so nothing was published.
    Unusable(FileProblem),
}

/// The scope label a file is written for: the settings prefix and the
/// process kind. A file with another label is foreign.
pub(crate) fn scope_label(base: &Base) -> String {
    format!(
        "{}:{}",
        base.catalog().prefix().as_str(),
        base.process_kind().name()
    )
}

/// The snapshot the file found at startup holds, judged against `base`.
pub(crate) fn load(base: &Base, bytes: &[u8]) -> Result<Snapshot, FileProblem> {
    let file = format::decode(bytes)?;
    if file.scope != scope_label(base) {
        return Err(FileProblem::Foreign);
    }
    let scope = base.validation_scope();
    // An entry written without a value is absent: the declared base serves.
    let rows = file.entries.iter().filter_map(|entry| match &entry.value {
        EntryValue::Absent => None,
        EntryValue::State(state) => Some(
            CandidateRow::new(
                &entry.key,
                &entry.kind,
                &entry.value_type,
                Version::new(entry.updated_version),
                state,
            )
            .written_for(DeclarationFingerprint::from_bytes(entry.fingerprint)),
        ),
    });
    let candidate = Candidate::from_last_good_file(&scope)
        .at(Version::new(file.version))
        .rows(rows);
    Snapshot::validate(&scope, candidate).map_err(FileProblem::Rejected)
}

/// What to write for a control-plane publication: the rows it was read
/// from, for every key this process declares and owns, under the
/// declaration's own tags. A held entry is written with a value only when it
/// carries an engaged switch from an earlier control-plane publication, as
/// that state; any other held entry is written without one, so the next load
/// serves the switch's safe state or the base for it.
pub(crate) fn file_of(
    base: &Base,
    snapshot: &Snapshot,
    rows: &[CandidateRow],
) -> Option<format::FileSnapshot> {
    match snapshot.provenance() {
        Provenance::ControlPlane => {}
        Provenance::LastGoodFile => return None,
    }
    let catalog = base.catalog();
    let owned: HashSet<FlagKey> = catalog.owned_by(base.process_kind()).ok()?.collect();
    let held: HashSet<&str> = snapshot.held().map(|(key, _)| key.as_str()).collect();
    let carried: Vec<_> = snapshot
        .carried()
        .filter(|carried| carried.provenance() == Provenance::ControlPlane)
        .collect();
    let mut entries = Vec::with_capacity(rows.len());
    for row in rows {
        let Some(declared) = catalog.declaration(row.key()) else {
            continue;
        };
        if !owned.contains(declared.key()) {
            continue;
        }
        let entry = |updated: Version, value: EntryValue| format::FileEntry {
            key: row.key().to_owned(),
            kind: declared.kind().as_str().to_owned(),
            value_type: declared.value_type().as_str().to_owned(),
            fingerprint: *declared.fingerprint().as_bytes(),
            updated_version: updated.get(),
            value,
        };
        if !held.contains(row.key()) {
            entries.push(entry(
                row.updated_version(),
                EntryValue::State(row.state_text().to_owned()),
            ));
        } else if let Some(kept) = carried.iter().find(|kept| kept.key().as_str() == row.key()) {
            entries.push(entry(
                kept.since(),
                EntryValue::State(format!(
                    "{{\"encoding\":1,\"switch\":\"{}\"}}",
                    kept.state().as_str()
                )),
            ));
        } else {
            entries.push(entry(row.updated_version(), EntryValue::Absent));
        }
    }
    Some(format::FileSnapshot {
        scope: scope_label(base),
        catalog_digest: *catalog.digest().as_bytes(),
        manifest_digest: *catalog.manifest_digest().as_bytes(),
        version: snapshot.version().get(),
        entries,
    })
}

/// Whether a process publishes and persists a last-good file.
#[derive(Clone, Debug)]
pub(crate) enum LastGood {
    /// No file: a restart during an outage serves the declared base.
    Off,
    /// The opened directory.
    File(Arc<SnapshotDirectory>),
}

#[cfg(test)]
mod tests;
