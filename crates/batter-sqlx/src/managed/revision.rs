//! Application-owned ordering, enforced by the managed apply owner.

use std::fmt;

/// A positive application-declared release order, representable as PostgreSQL bigint.
///
/// Revisions order manifests for one ledger. They are not timestamps or SQLx
/// migration versions. A changed manifest needs a higher revision, including a
/// deliberate rollback to earlier source. Reusing a revision with different
/// content is refused. Rust validates the number; PostgreSQL enforces ordering.
///
/// ```
/// use batter_sqlx::managed::ManagedRevision;
/// let revision = ManagedRevision::new(12).expect("positive bigint revision");
/// assert_eq!(revision.get(), 12);
/// assert!(ManagedRevision::new(0).is_none());
/// ```
///
/// # Choosing a revision
///
/// For one ledger the revision must never decrease, and one revision must
/// name one content digest. A single hand-written constant is one line that
/// every set-changing branch rewrites: branches that pick different numbers
/// conflict on it, branches that raise it to the same number merge without a
/// conflict, and either way the merged number can name a set neither branch
/// built. A frozen base plus a count of append-only records avoids the shared
/// line: each set change adds one uniquely named record file, so a merge holds
/// both parents' records and its revision is above both.
///
/// ```
/// use batter_sqlx::managed::ManagedRevision;
///
/// /// The last hand-chosen revision, or zero for a new ledger. Never changes.
/// const BASE: u64 = 41;
/// const fn revision(records: usize) -> ManagedRevision {
///     ManagedRevision::new(BASE + records as u64).expect("a positive bigint revision")
/// }
/// // Two branches from a tree with three records each add one; the merge has both.
/// const BRANCH: ManagedRevision = revision(4);
/// const MERGED: ManagedRevision = revision(5);
/// assert!(BRANCH < MERGED);
/// assert_eq!(MERGED.get(), 46);
/// ```
///
/// Batter neither reads such records nor checks that they are append-only;
/// that stays with the application's build and review. The recipe assumes one
/// releasing line per ledger: two unmerged branches share a revision with
/// different content, which apply refuses, and a stale branch holding more
/// records than the releasing line is a higher revision, which apply accepts.
/// See "Choosing a revision" in this package's README.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ManagedRevision(i64);

impl ManagedRevision {
    /// Return `None` for zero or a value greater than `i64::MAX`.
    pub const fn new(value: u64) -> Option<Self> {
        if value == 0 || value > i64::MAX as u64 {
            None
        } else {
            Some(Self(value as i64))
        }
    }

    /// Return the positive revision for release evidence.
    pub const fn get(self) -> i64 {
        self.0
    }
}

/// A managed release refused before any managed object changes or application callback.
/// SQLx migrations already committed by the migrator are not rolled back.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ManagedReleaseError {
    /// An installed newer release must not be overwritten by this caller.
    Downgrade {
        /// Revision declared by this caller.
        requested: ManagedRevision,
        /// Newer revision retained by the ledger.
        installed: ManagedRevision,
    },
    /// One revision cannot name two different manifest digests.
    RevisionConflict {
        /// Revision already bound to different content.
        revision: ManagedRevision,
    },
    /// Upgrade the legacy ledger with its currently recorded manifest first.
    /// A different manifest cannot establish the unversioned ledger's baseline.
    LegacyBaselineRequired,
    /// A versioned ledger has missing or invalid release evidence.
    InvalidLedger,
}

impl fmt::Display for ManagedReleaseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Downgrade { .. } => "managed release is older than the installed release",
            Self::RevisionConflict { .. } => "managed revision already names different content",
            Self::LegacyBaselineRequired => {
                "legacy managed ledger requires its recorded manifest baseline"
            }
            Self::InvalidLedger => "managed ledger has invalid release evidence",
        })
    }
}

impl std::error::Error for ManagedReleaseError {}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn revisions_are_positive_bigints() {
        assert!(ManagedRevision::new(0).is_none());
        assert!(ManagedRevision::new(i64::MAX as u64 + 1).is_none());
        assert_eq!(
            ManagedRevision::new(i64::MAX as u64).unwrap().get(),
            i64::MAX
        );
        assert!(ManagedRevision::new(1).unwrap() < ManagedRevision::new(2).unwrap());
    }
}
