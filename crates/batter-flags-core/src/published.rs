//! What a loader publishes: one immutable record, advanced by one publisher.
//!
//! A snapshot is only as good as what vouches for it: where it was read from,
//! which version and digest were applied, when the control plane last
//! confirmed it and why the latest read was refused. A loader that keeps
//! those apart, publishing the snapshot and then writing a timestamp, leaves a
//! gap between the two writes in which another thread pairs a new snapshot
//! with an old confirmation. The record keeps them together: every read
//! replaces the whole of it, and a view is built from one record, so the
//! evidence that decides freshness is always the evidence of the snapshot being
//! served.
//!
//! A [`Publisher`] owns the record through its whole lifecycle. It starts
//! [`Unpublished`], or restored from the last-good file, and every later read
//! goes through [`Publisher::judge`] and [`Judged::publish`]. The publisher
//! carries the kill-switch states of what it serves into each read, picks where
//! age counts from and applies the [`Authority`] rules, so none of those is a
//! step its caller can forget or reorder.
//!
//! Core reads no clock, so the evidence is a type `C` the loader chooses (a
//! monotonic instant) and freshness is asked of it through a closure.

use std::{fmt, sync::Arc};

use crate::{
    authority::{Authority, Refusal, Standing, Transition},
    candidate::{Candidate, CandidateRow},
    decision::Freshness,
    snapshot::{Provenance, Snapshot, SnapshotRejection, ValidationScope},
    version::Version,
};

mod views;

/// The one owner of what a process publishes, and the only way to change it.
///
/// It is not `Clone`: a loader holds one and every reader sees the
/// [`Record`] it hands out, so no stale copy of a record can be advanced.
///
/// ```compile_fail,E0599
/// fn twice(publisher: batter_flags_core::low_level::Publisher<u64>) {
///     let _copy = publisher.clone();
/// }
/// ```
///
/// ```
/// use batter_flags_core::{
///     CatalogBuilder, Freshness, PrePublication, SafeState, SettingsPrefix, StaleRule,
///     SwitchState, Version,
///     low_level::{BaseBuilder, Candidate, CandidateRow, Disposition, Publisher, Record},
/// };
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let (mut catalog, serving, _) = CatalogBuilder::new(SettingsPrefix::new("FLAG_")?, "serving", "hosted")?;
/// let halt = catalog
///     .kill_switch("checkout_halt", SafeState::Released, StaleRule::HoldLastKnown)
///     .declare()?;
/// let base = BaseBuilder::new(catalog.build(), serving)?.finish();
/// let scope = base.validation_scope();
///
/// // The evidence here is a tick count; a loader would use a monotonic instant.
/// let mut publisher = Publisher::new(&scope, 0_u64);
/// let engaged = Candidate::new(&scope, Version::new(5))
///     .row(CandidateRow::switch(&halt, SwitchState::Engaged, Version::new(5)));
/// assert_eq!(publisher.judge(engaged).publish(100), Disposition::Applied);
///
/// // A later read whose switch row cannot be decoded is held, and the publisher
/// // carries the engaged state into it: holding never releases a switch.
/// let held = Candidate::new(&scope, Version::new(6)).row(CandidateRow::new(
///     "checkout_halt", "switch", "switch", Version::new(6), r#"{"encoding":9}"#,
/// ));
/// assert_eq!(publisher.judge(held).publish(130), Disposition::Applied);
///
/// // A lower version is refused and nothing it holds is served.
/// let regressed = Candidate::new(&scope, Version::new(4));
/// assert!(matches!(publisher.judge(regressed).publish(140), Disposition::Refused(_)));
///
/// let view = base.record_view(publisher.record(), PrePublication::Unready, |at| 150 - at <= 60)?;
/// assert_eq!(view.freshness(), Freshness::Fresh);
/// assert_eq!(view.switch(&halt).value(), SwitchState::Engaged);
/// assert!(matches!(publisher.record(), Record::Published(_)));
/// # Ok(())
/// # }
/// ```
pub struct Publisher<C> {
    scope: ValidationScope,
    record: Record<C>,
}

/// What a process serves: nothing yet, or one published snapshot.
///
/// Records are handed out by a [`Publisher`]; cloning one shares what it
/// serves and grants nothing, because only the publisher advances.
#[derive(Clone, Debug)]
pub enum Record<C> {
    /// Nothing is published; views serve the declared base.
    Unpublished(Unpublished<C>),
    /// A snapshot is published.
    Published(Published<C>),
}

/// A record before its first publication.
#[derive(Clone, Debug)]
pub struct Unpublished<C> {
    registered: C,
    refusal: Option<Refusal>,
}

/// The snapshot a process serves, what it stands on and what confirmed it.
///
/// Only a [`Publisher`] makes one: from the last-good file with
/// [`Publisher::restore`], or from a read with [`Judged::publish`], which
/// applies the [`Authority`] rules, so a regressed, drifted or misplaced
/// snapshot can neither replace nor renew what is served.
///
/// Migration: `Published::first`, `Published::next` and `Published::rejected`
/// are no longer public; a loader keeps a [`Publisher`] and advances it with
/// [`Publisher::judge`] and [`Judged::publish`] instead.
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

/// What restoring the last-good file did.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[must_use = "a restore says whether the file was published"]
pub enum Restored {
    /// The file's snapshot is the first publication.
    Published,
    /// The file's entries were structurally corrupt, so nothing is published
    /// and the record keeps the refusal.
    Rejected(SnapshotRejection),
}

/// A read the publisher has judged, before anything changed.
///
/// [`publish`](Self::publish) records the outcome; dropping it records
/// nothing, so a loader whose deadline passed while the read was judged
/// leaves the record exactly as it was. It borrows the publisher, so no other
/// read can be published in between.
#[must_use = "a judged read changes nothing until it is published"]
pub struct Judged<'p, C> {
    publisher: &'p mut Publisher<C>,
    verdict: Verdict,
}

/// What judging a read found, before publication.
enum Verdict {
    Refuse(Refusal),
    Valid(Snapshot),
}

impl<C: Clone> Publisher<C> {
    /// A publisher that has published nothing.
    ///
    /// `registered` is where age counts from until something is published,
    /// and for a last-good file, which carries no clock of its own.
    pub fn new(scope: &ValidationScope, registered: C) -> Self {
        Self {
            scope: scope.clone(),
            record: Record::Unpublished(Unpublished {
                registered,
                refusal: None,
            }),
        }
    }

    /// A publisher whose first publication is the last-good file read at
    /// startup, at `version` with `rows`.
    ///
    /// The file confirms nothing, so its age counts from `registered`. A file
    /// is only ever restored here: offered to [`judge`](Self::judge) later, it
    /// is refused.
    pub fn restore(
        scope: &ValidationScope,
        registered: C,
        version: Version,
        rows: impl IntoIterator<Item = CandidateRow>,
    ) -> (Self, Restored) {
        let candidate = Candidate::from_last_good_file(scope, version).rows(rows);
        let (record, restored) = match Snapshot::validate(candidate) {
            Ok(snapshot) => (
                Record::Published(Published::first(Arc::new(snapshot), registered)),
                Restored::Published,
            ),
            Err(rejection) => (
                Record::Unpublished(Unpublished {
                    registered,
                    refusal: Some(Refusal::Rejected(rejection)),
                }),
                Restored::Rejected(rejection),
            ),
        };
        (
            Self {
                scope: scope.clone(),
                record,
            },
            restored,
        )
    }

    /// Judges one control-plane read against what is served.
    ///
    /// The publisher refuses a candidate read for another scope, one read
    /// from the last-good file and, once a control-plane version is applied,
    /// one below it, whatever its rows hold. Otherwise it carries the
    /// kill-switch states it serves into the candidate, so a held switch keeps
    /// its state, and validates it. Nothing changes until the result is
    /// [published](Judged::publish).
    pub fn judge(&mut self, mut candidate: Candidate) -> Judged<'_, C> {
        let verdict = if !Arc::ptr_eq(&candidate.base, &self.scope.base) {
            Verdict::Refuse(Refusal::Rejected(SnapshotRejection::ForeignScope))
        } else if candidate.provenance == Provenance::LastGoodFile {
            Verdict::Refuse(Refusal::FileAfterPublication)
        } else if let Some(refusal) = self.regression(&candidate) {
            Verdict::Refuse(refusal)
        } else {
            match &self.record {
                // A first publication stands on nothing carried.
                Record::Unpublished(_) => candidate.carry_nothing(),
                Record::Published(published) => candidate.carry_from(&published.snapshot),
            }
            match Snapshot::validate(candidate) {
                Ok(snapshot) => Verdict::Valid(snapshot),
                Err(rejection) => Verdict::Refuse(Refusal::Rejected(rejection)),
            }
        };
        Judged {
            publisher: self,
            verdict,
        }
    }

    /// The record readers serve now.
    pub fn record(&self) -> &Record<C> {
        &self.record
    }

    /// A control-plane read below the applied version, which is refused
    /// before its rows are looked at.
    fn regression(&self, candidate: &Candidate) -> Option<Refusal> {
        let Record::Published(published) = &self.record else {
            return None;
        };
        let observed = candidate.version;
        match published.authority.standing() {
            Standing::ControlPlane {
                version: applied, ..
            } if observed < applied => Some(Refusal::Regressed { applied, observed }),
            Standing::Unpublished | Standing::Persisted { .. } | Standing::ControlPlane { .. } => {
                None
            }
        }
    }
}

impl<C: Clone> Judged<'_, C> {
    /// Records what the read did, with `at` as the evidence it vouches for.
    ///
    /// A snapshot that applies replaces the record and counts its age from
    /// `at`; one that confirms keeps the snapshot and moves the age origin to
    /// `at`; one that is refused keeps both and records why. Applying or
    /// confirming clears an earlier refusal, and nothing else does.
    pub fn publish(self, at: C) -> Disposition {
        let Self { publisher, verdict } = self;
        let (record, disposition) = match (&publisher.record, verdict) {
            (Record::Unpublished(unpublished), Verdict::Refuse(refusal)) => (
                Record::Unpublished(Unpublished {
                    registered: unpublished.registered.clone(),
                    refusal: Some(refusal),
                }),
                Disposition::Refused(refusal),
            ),
            (Record::Unpublished(_), Verdict::Valid(snapshot)) => (
                Record::Published(Published::first(Arc::new(snapshot), at)),
                Disposition::Applied,
            ),
            (Record::Published(published), Verdict::Refuse(refusal)) => (
                Record::Published(published.refused(refusal)),
                Disposition::Refused(refusal),
            ),
            (Record::Published(published), Verdict::Valid(snapshot)) => {
                let (next, disposition) = published.next(Arc::new(snapshot), at);
                (Record::Published(next), disposition)
            }
        };
        publisher.record = record;
        disposition
    }
}

impl<C> Record<C> {
    /// What the record stands on.
    pub fn authority(&self) -> Authority {
        match self {
            Self::Unpublished(_) => Authority::unpublished(),
            Self::Published(published) => published.authority,
        }
    }

    /// The snapshot served, once one is published.
    pub fn snapshot(&self) -> Option<&Arc<Snapshot>> {
        match self {
            Self::Unpublished(_) => None,
            Self::Published(published) => Some(&published.snapshot),
        }
    }

    /// The evidence age is counted from: registration until something is
    /// published, then the record's own age origin.
    pub fn age_origin(&self) -> &C {
        match self {
            Self::Unpublished(unpublished) => &unpublished.registered,
            Self::Published(published) => &published.age_origin,
        }
    }

    /// Why the latest read was refused, until a read applies or confirms.
    pub fn last_refusal(&self) -> Option<Refusal> {
        match self {
            Self::Unpublished(unpublished) => unpublished.refusal,
            Self::Published(published) => published.refusal,
        }
    }

    /// Whether the record is fresh, judged by `recent` over its age origin.
    pub fn freshness(&self, recent: impl FnOnce(&C) -> bool) -> Freshness {
        if recent(self.age_origin()) {
            Freshness::Fresh
        } else {
            Freshness::Stale
        }
    }
}

impl<C> Unpublished<C> {
    /// Where age counts from: the moment the publisher was made.
    pub fn registered(&self) -> &C {
        &self.registered
    }

    /// Why the latest read was refused, if one was.
    pub fn last_refusal(&self) -> Option<Refusal> {
        self.refusal
    }
}

impl<C: Clone> Published<C> {
    /// The first publication: the last-good file at startup, or the first
    /// control-plane read.
    pub(crate) fn first(snapshot: Arc<Snapshot>, at: C) -> Self {
        Self {
            authority: Authority::first(&snapshot),
            snapshot,
            age_origin: at,
            refusal: None,
        }
    }

    /// The record after a later valid read, and what that read did.
    fn next(&self, snapshot: Arc<Snapshot>, at: C) -> (Self, Disposition) {
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

    fn refused(&self, refusal: Refusal) -> Self {
        Self {
            refusal: Some(refusal),
            ..self.clone()
        }
    }
}

impl<C> Published<C> {
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

impl<C: fmt::Debug> fmt::Debug for Publisher<C> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Publisher")
            .field("record", &self.record)
            .finish_non_exhaustive()
    }
}

impl<C> fmt::Debug for Judged<'_, C> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let verdict = match &self.verdict {
            Verdict::Refuse(refusal) => format!("Refuse({refusal:?})"),
            Verdict::Valid(snapshot) => format!("Valid(version {})", snapshot.version().get()),
        };
        formatter
            .debug_struct("Judged")
            .field("verdict", &verdict)
            .finish_non_exhaustive()
    }
}
