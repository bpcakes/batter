//! What a loader publishes: one immutable record.
//!
//! A snapshot is only as good as what vouches for it: where it was read from,
//! which version and digest were applied, when the control plane last
//! confirmed it and why the latest read was refused. A loader that keeps
//! those apart, publishing the snapshot and then writing a timestamp, leaves a
//! gap between the two writes in which another thread pairs a new snapshot
//! with an old confirmation. The record keeps them together: every poll
//! replaces the whole of it, and a view is built from one record, so the
//! evidence that decides freshness is always the evidence of the snapshot being
//! served.
//!
//! Core reads no clock, so the evidence is a type `C` the loader chooses (a
//! monotonic instant) and freshness is asked of it through a closure.

use std::sync::Arc;

use crate::{
    authority::{Authority, Refusal, Transition},
    decision::Freshness,
    snapshot::{Provenance, Snapshot, SnapshotRejection},
    view::{SnapshotView, ViewError},
};

/// The snapshot a process serves, what it stands on and what confirmed it.
///
/// The only ways to a new record are [`first`](Self::first),
/// [`next`](Self::next) and [`rejected`](Self::rejected), and `next` goes
/// through [`Authority::consider`], so a regressed, drifted or misplaced
/// snapshot can neither replace nor renew what is served.
///
/// ```
/// use std::sync::Arc;
///
/// use batter_flags_core::{
///     CatalogBuilder, Freshness, SettingsPrefix, Version,
///     low_level::{BaseBuilder, Candidate, Disposition, Published, Snapshot},
/// };
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let mut catalog = CatalogBuilder::new(SettingsPrefix::new("FLAG_")?);
/// let serving = catalog.process_kind("serving")?;
/// catalog.environment_class("hosted")?;
/// let base = BaseBuilder::new(catalog.build()?, serving)?.finish();
/// let scope = base.validation_scope();
/// let read = |at: u64| -> Result<Arc<Snapshot>, Box<dyn std::error::Error>> {
///     Ok(Arc::new(Snapshot::validate(&scope, Candidate::new(&scope).at(Version::new(at)))?))
/// };
///
/// // The evidence here is a tick count; a loader would use a monotonic instant.
/// let first = Published::first(read(5)?, 100_u64);
/// let (confirmed, disposition) = first.next(read(5)?, 130);
/// assert_eq!(disposition, Disposition::Confirmed);
///
/// // The view and its freshness come from the same record: confirmed at 130,
/// // so at tick 150 it is recent, and at 300 it is not.
/// let view = base.published_view(&confirmed, |at| 150 - at <= 60)?;
/// assert_eq!(view.freshness(), Freshness::Fresh);
/// let view = base.published_view(&confirmed, |at| 300 - at <= 60)?;
/// assert_eq!(view.freshness(), Freshness::Stale);
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug)]
pub struct Published<C> {
    snapshot: Arc<Snapshot>,
    authority: Authority,
    age_origin: C,
    refusal: Option<Refusal>,
}

/// What a read did to the record.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Disposition {
    /// The snapshot was published.
    Applied,
    /// The snapshot already served was confirmed again.
    Confirmed,
    /// Nothing changed but the recorded refusal.
    Refused(Refusal),
}

impl<C: Clone> Published<C> {
    /// The first publication: the last-good file at startup, or the first
    /// control-plane read.
    ///
    /// `at` is where age is counted from. For a control-plane snapshot that is
    /// the read that confirmed it. A file confirms nothing and carries no
    /// clock, so for a file it is the moment the process registered.
    pub fn first(snapshot: Arc<Snapshot>, at: C) -> Self {
        Self {
            authority: Authority::first(&snapshot),
            snapshot,
            age_origin: at,
            refusal: None,
        }
    }

    /// The record after a later read, and what that read did.
    ///
    /// A snapshot that applies replaces the record; one that confirms keeps the
    /// snapshot and moves the age origin to `at`; one that is refused keeps
    /// both and records why. Applying or confirming clears an earlier refusal,
    /// and nothing else does.
    pub fn next(&self, snapshot: Arc<Snapshot>, at: C) -> (Self, Disposition) {
        match self.authority.consider(&snapshot) {
            Transition::Apply(authority) => (
                Self {
                    snapshot,
                    authority,
                    age_origin: at,
                    refusal: None,
                },
                Disposition::Applied,
            ),
            Transition::Confirm => (
                Self {
                    snapshot: Arc::clone(&self.snapshot),
                    authority: self.authority,
                    age_origin: at,
                    refusal: None,
                },
                Disposition::Confirmed,
            ),
            Transition::Refuse(refusal) => (self.refused(refusal), Disposition::Refused(refusal)),
        }
    }

    /// The record after a candidate that was structurally corrupt: the
    /// snapshot, authority and age origin are untouched.
    pub fn rejected(&self, rejection: SnapshotRejection) -> Self {
        self.refused(Refusal::Rejected(rejection))
    }

    fn refused(&self, refusal: Refusal) -> Self {
        Self {
            refusal: Some(refusal),
            ..self.clone()
        }
    }

    /// The snapshot served.
    pub fn snapshot(&self) -> &Arc<Snapshot> {
        &self.snapshot
    }

    /// What the snapshot stands on.
    pub fn authority(&self) -> Authority {
        self.authority
    }

    /// The evidence age is counted from.
    pub fn age_origin(&self) -> &C {
        &self.age_origin
    }

    /// Why the latest read was refused, until a read applies or confirms.
    pub fn last_refusal(&self) -> Option<Refusal> {
        self.refusal
    }

    /// Whether the record is fresh, judged by `recent` over this record's own
    /// age origin.
    pub fn freshness(&self, recent: impl FnOnce(&C) -> bool) -> Freshness {
        if recent(&self.age_origin) {
            Freshness::Fresh
        } else {
            Freshness::Stale
        }
    }
}

impl crate::base::Base {
    /// A view of a published record, with the source its snapshot was read from
    /// and the freshness `recent` finds in that record's own age origin.
    ///
    /// # Errors
    ///
    /// [`ViewError::ForeignSnapshot`] when the record's snapshot was validated
    /// for another base.
    pub fn published_view<C: Clone>(
        &self,
        published: &Published<C>,
        recent: impl FnOnce(&C) -> bool,
    ) -> Result<SnapshotView, ViewError> {
        let freshness = published.freshness(recent);
        match published.snapshot.provenance() {
            Provenance::ControlPlane => self.control_plane_view(&published.snapshot, freshness),
            Provenance::LastGoodFile => self.persisted_file_view(&published.snapshot, freshness),
        }
    }
}
