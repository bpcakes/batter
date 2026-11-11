//! Private transaction decisions. Public outcomes require native completion proof.
use crate::{Deferral, Refusal, StoredState, UndeclaredBasis, WriteOutcome};
use batter_flags_core::Version;
use batter_sqlx::{PgCommitConfirmed, PgRollbackConfirmed};
use std::num::NonZeroUsize;

pub(crate) enum Prepared {
    Commit(PendingWrite),
    Rollback(RollbackDecision),
}
pub(crate) enum RollbackDecision {
    Unchanged(Version),
    RevisionMismatch { current: Version, head: Version },
    Refused(Refusal),
}
impl RollbackDecision {
    pub(crate) fn completed(self, _: PgRollbackConfirmed) -> WriteOutcome {
        match self {
            Self::Unchanged(version) => WriteOutcome::Unchanged { version },
            Self::RevisionMismatch { current, head } => {
                WriteOutcome::RevisionMismatch { current, head }
            }
            Self::Refused(refusal) => WriteOutcome::Refused(refusal),
        }
    }
}
pub(crate) enum DeclarationEvidence {
    Live {
        previous: Option<StoredState>,
        count: NonZeroUsize,
    },
    Undeclared(UndeclaredBasis),
}
pub(crate) struct PendingWrite {
    pub(crate) version: Version,
    pub(crate) declaration: DeclarationEvidence,
    pub(crate) deferral: Deferral,
}
impl PendingWrite {
    pub(crate) fn committed(self, _: PgCommitConfirmed) -> WriteOutcome {
        match self.declaration {
            DeclarationEvidence::Live { previous, count } => WriteOutcome::Written {
                version: self.version,
                previous,
                live_declaring: count.get(),
                deferral: self.deferral,
            },
            DeclarationEvidence::Undeclared(basis) => WriteOutcome::WrittenUndeclared {
                version: self.version,
                basis,
                deferral: self.deferral,
            },
        }
    }
}
