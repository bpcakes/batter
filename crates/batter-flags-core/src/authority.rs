//! The standing of what a process serves, and the transitions that change it.
//!
//! A loader publishes snapshots from two sources with different standing: the
//! last-good file, read once before anything else, and the control plane.
//! The rules that follow are pure, with no clock and no I/O, so the loader
//! cannot get them subtly wrong while it is also doing I/O:
//!
//! - A file snapshot can only be the first publication. It confirms nothing
//!   and is never an applied version.
//! - The first control-plane snapshot is applied even at or below the file's
//!   version: a file's version says nothing about the control plane's.
//! - After that, control-plane versions never decrease. A lower version is
//!   refused, a repeated version with the same digest only confirms, and a
//!   repeated version with another digest is drift and confirms nothing.
//!
//! [`Authority`] holds the standing and [`consider`](Authority::consider) is
//! the only way to a new one, so an applied version and digest are always those
//! of a snapshot that was judged, never assembled by hand.

use crate::{
    digest::SnapshotDigest,
    snapshot::{Provenance, Snapshot, SnapshotRejection},
    version::Version,
};

/// What a process serves and what it was confirmed against.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Authority(Standing);

/// What a record stands on, for status and diagnostics.
///
/// Reading a standing grants nothing: only a
/// [`Publisher`](crate::low_level::Publisher) judging a read makes one.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Standing {
    /// Nothing has been published.
    Unpublished,
    /// The last-good file was published; the control plane has not been.
    Persisted {
        /// The file's version.
        version: Version,
    },
    /// A control-plane snapshot was applied.
    ControlPlane {
        /// The applied version.
        version: Version,
        /// The digest of the candidate as read at that version.
        digest: SnapshotDigest,
    },
}

/// What a loader does with a snapshot it read.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[must_use = "a transition says whether to publish and whether to renew freshness"]
pub(crate) enum Transition {
    /// Publish the snapshot. The authority is the standing it now has, and
    /// freshness renews only when that standing is the control plane's.
    Apply(Authority),
    /// The snapshot already served is confirmed unchanged: renew freshness and
    /// publish nothing.
    Confirm,
    /// Keep serving what is served, renew nothing and say why.
    Refuse(Refusal),
}

/// Why a snapshot was not published or confirmed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Refusal {
    /// The read is below the applied control-plane version.
    Regressed {
        /// The applied version.
        applied: Version,
        /// The version just read.
        observed: Version,
    },
    /// The read is at the applied version with another digest: the control
    /// plane's content changed without its version moving.
    DigestDrift {
        /// The version both reads carry.
        version: Version,
    },
    /// A snapshot read from the file was offered after the publisher started.
    /// The file is read once, first, through
    /// [`Publisher::restore`](crate::low_level::Publisher::restore); a loader
    /// that reaches this has a bug.
    FileAfterPublication,
    /// The candidate was structurally corrupt, so it never became a snapshot to
    /// judge. A [`Publisher`](crate::low_level::Publisher) records it as the
    /// latest refusal, before or after the first publication.
    Rejected(SnapshotRejection),
}

impl Authority {
    /// The standing of a process that has published nothing.
    pub(crate) const fn unpublished() -> Self {
        Self(Standing::Unpublished)
    }

    /// What this authority stands on.
    pub(crate) fn standing(&self) -> Standing {
        self.0
    }

    /// The standing a first publication of `snapshot` has.
    pub(crate) fn first(snapshot: &Snapshot) -> Self {
        let version = snapshot.version();
        Self(match snapshot.provenance() {
            Provenance::LastGoodFile => Standing::Persisted { version },
            Provenance::ControlPlane => Standing::ControlPlane {
                version,
                digest: snapshot.digest(),
            },
        })
    }

    /// Judges `snapshot` against the standing, by its provenance, version and
    /// digest.
    pub(crate) fn consider(&self, snapshot: &Snapshot) -> Transition {
        let (version, digest) = (snapshot.version(), snapshot.digest());
        match (self.0, snapshot.provenance()) {
            (Standing::Unpublished, _) => Transition::Apply(Self::first(snapshot)),
            (_, Provenance::LastGoodFile) => Transition::Refuse(Refusal::FileAfterPublication),
            (Standing::Persisted { .. }, Provenance::ControlPlane) => {
                Transition::Apply(Self(Standing::ControlPlane { version, digest }))
            }
            (
                Standing::ControlPlane {
                    version: applied,
                    digest: applied_digest,
                },
                Provenance::ControlPlane,
            ) => {
                if version < applied {
                    Transition::Refuse(Refusal::Regressed {
                        applied,
                        observed: version,
                    })
                } else if version > applied {
                    Transition::Apply(Self(Standing::ControlPlane { version, digest }))
                } else if digest == applied_digest {
                    Transition::Confirm
                } else {
                    Transition::Refuse(Refusal::DigestDrift { version })
                }
            }
        }
    }
}
