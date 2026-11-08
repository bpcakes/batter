//! The reader: how registered flags turn into one view per unit of work.
//!
//! A view is a frozen evaluation context: the base, at most one snapshot, a
//! source and the freshness its publisher computed. The reader is the
//! publisher. In declared mode it has nothing to publish; for injected test
//! flags it holds one fixed snapshot; behind a control plane it follows one
//! immutable [`Published`] record, replaced whole by a single writer, so the
//! evidence that decides freshness is always the evidence of the snapshot
//! being served.
//!
//! Publication is a `tokio::sync::watch` channel. Capturing a view clones the
//! `Arc` of the current record and releases the channel's read lock before it
//! evaluates anything, and the writer drops the replaced record after the lock
//! is released. Tokio's watch lock ignores poisoning, so a capture never
//! panics. Age is measured with `tokio::time::Instant`, so paused-time tests
//! drive staleness.
//!
//! Everything here is crate-private. The live read is public only through the
//! disclosed, non-canonical [`low_level::capture_now`](crate::low_level::capture_now).

use std::{sync::Arc, time::Duration};

use batter_flags_core::{
    Freshness, PrePublication, SnapshotView,
    low_level::{
        Base, Disposition, Provenance, Published, Refusal, Snapshot, SnapshotRejection, Standing,
        ViewError,
    },
};
use tokio::{sync::watch, time::Instant};

/// What the control plane has published, if anything.
type Record = Option<Arc<Published<Instant>>>;

/// Where a reader's views come from.
enum Mode {
    /// Declared mode: the settings read at startup, no control plane.
    Declared,
    /// Injected test flags: one fixed snapshot and freshness.
    Test {
        snapshot: Arc<Snapshot>,
        freshness: Freshness,
    },
    /// A control plane: the record a loader publishes.
    #[cfg_attr(
        not(feature = "postgres"),
        allow(
            dead_code,
            reason = "only the control plane, behind `postgres`, publishes outside tests"
        )
    )]
    Live(Live),
}

struct Live {
    receiver: watch::Receiver<Record>,
    pre_publication: PrePublication,
    maximum_age: Duration,
    /// Age counts from registration until something is published.
    registered: Instant,
}

/// The shared half of the reader: what every clone of the flags captures from.
pub(crate) struct Reader {
    base: Base,
    mode: Mode,
}

/// The only way to publish: held by the component that loads snapshots, and
/// dropped when it ends.
///
/// It owns the publication protocol instead of leaving it to its caller. What
/// readers serve is a [`Published`] record, and the writer derives every new
/// record from the one being served, through [`Published::next`], so core's
/// rules apply to every read: a version never goes backwards, a repeated version
/// with other content is drift, and a file read can only be first. Nothing can
/// publish a record the writer did not derive. The mutating methods take
/// `&mut self`, so there is one writer at a time, and the record it derives
/// from cannot change between reading it and replacing it.
#[cfg_attr(
    not(feature = "postgres"),
    allow(
        dead_code,
        reason = "only the control plane, behind `postgres`, publishes outside tests"
    )
)]
pub(crate) struct Writer {
    sender: watch::Sender<Record>,
    /// Where age counts from for what carries no clock of its own.
    registered: Instant,
}

impl Reader {
    /// A reader for declared mode.
    pub(crate) fn declared(base: Base) -> Self {
        Self {
            base,
            mode: Mode::Declared,
        }
    }

    /// A reader of one fixed snapshot, as the fixture path serves it.
    pub(crate) fn test(base: Base, snapshot: Arc<Snapshot>, freshness: Freshness) -> Self {
        Self {
            base,
            mode: Mode::Test {
                snapshot,
                freshness,
            },
        }
    }

    /// A reader over a control plane, and the writer that publishes to it.
    ///
    /// `registered` is where age counts from until the first publication,
    /// and `maximum_age` is how long a confirmation keeps a view fresh.
    #[cfg_attr(
        not(feature = "postgres"),
        allow(
            dead_code,
            reason = "only the control plane, behind `postgres`, publishes outside tests"
        )
    )]
    pub(crate) fn live(
        base: Base,
        pre_publication: PrePublication,
        maximum_age: Duration,
        registered: Instant,
    ) -> (Writer, Self) {
        let (sender, receiver) = watch::channel(None);
        let reader = Self {
            base,
            mode: Mode::Live(Live {
                receiver,
                pre_publication,
                maximum_age,
                registered,
            }),
        };
        (Writer { sender, registered }, reader)
    }

    /// The base every view of this reader is over.
    pub(crate) fn base(&self) -> &Base {
        &self.base
    }

    /// The digest of the fixed snapshot of a test reader, which no public
    /// surface shows: it covers every row exactly as written, versions included.
    #[cfg(test)]
    pub(crate) fn fixture_digest(&self) -> Option<batter_flags_core::SnapshotDigest> {
        match &self.mode {
            Mode::Test { snapshot, .. } => Some(snapshot.digest()),
            Mode::Declared | Mode::Live(_) => None,
        }
    }

    /// One view, frozen now. Never panics and does no I/O.
    pub(crate) fn capture(&self) -> SnapshotView {
        self.capture_at(Instant::now())
    }

    /// One view, with freshness judged at `now`.
    ///
    /// A snapshot that no longer matches this reader's base cannot happen by
    /// construction. If it ever did, the view would be not-loaded and
    /// stale, so every decision reports an error and the broken state is loud
    /// instead of silently serving defaults.
    pub(crate) fn capture_at(&self, now: Instant) -> SnapshotView {
        let view = match &self.mode {
            Mode::Declared => Ok(self.base.declared_view()),
            Mode::Test {
                snapshot,
                freshness,
            } => self.base.test_view(snapshot, *freshness),
            Mode::Live(live) => live.capture(&self.base, now),
        };
        view.unwrap_or_else(|_: ViewError| {
            self.base
                .not_loaded_view(PrePublication::Unready, Freshness::Stale)
        })
    }

    /// Whether the control plane's writer has been dropped, so nothing will
    /// ever publish again. Declared and test readers have no writer to lose.
    #[cfg_attr(
        not(feature = "postgres"),
        allow(
            dead_code,
            reason = "only the control plane, behind `postgres`, publishes outside tests"
        )
    )]
    pub(crate) fn writer_gone(&self) -> bool {
        match &self.mode {
            Mode::Live(live) => live.receiver.has_changed().is_err(),
            Mode::Declared | Mode::Test { .. } => false,
        }
    }
}

#[cfg_attr(
    not(feature = "postgres"),
    allow(
        dead_code,
        reason = "only the control plane, behind `postgres`, publishes outside tests"
    )
)]
/// What a control-plane reader serves, for status and presence.
pub(crate) enum Publication {
    /// Nothing is published, and this long has passed since registration.
    Unpublished { since_registration: Duration },
    /// A record is published: what it stands on, and whether its age is
    /// within the maximum.
    Published { standing: Standing, fresh: bool },
}

#[cfg_attr(
    not(feature = "postgres"),
    allow(
        dead_code,
        reason = "only the control plane, behind `postgres`, publishes outside tests"
    )
)]
impl Reader {
    /// The record being served, behind a control plane.
    pub(crate) fn record(&self) -> Record {
        match &self.mode {
            Mode::Live(live) => live.receiver.borrow().clone(),
            Mode::Declared | Mode::Test { .. } => None,
        }
    }

    /// Whether anything, a control-plane read or a file, is published.
    pub(crate) fn has_publication(&self) -> bool {
        self.record().is_some()
    }

    /// What is served at `now`. Declared and test readers have no control
    /// plane and report as never published.
    pub(crate) fn publication(&self, now: Instant) -> Publication {
        match &self.mode {
            Mode::Live(live) => match live.receiver.borrow().clone() {
                Some(record) => Publication::Published {
                    standing: record.authority().standing(),
                    fresh: live.recent(*record.age_origin(), now),
                },
                None => Publication::Unpublished {
                    since_registration: now.saturating_duration_since(live.registered),
                },
            },
            Mode::Declared | Mode::Test { .. } => Publication::Unpublished {
                since_registration: Duration::ZERO,
            },
        }
    }
}

impl Live {
    fn capture(&self, base: &Base, now: Instant) -> Result<SnapshotView, ViewError> {
        // Clone the `Arc`; the read lock is released at the end of the
        // statement, before any evaluation.
        let record: Record = self.receiver.borrow().clone();
        match record {
            Some(published) => base.published_view(&published, |origin| self.recent(*origin, now)),
            None => Ok(
                base.not_loaded_view(self.pre_publication, self.freshness(self.registered, now))
            ),
        }
    }

    fn recent(&self, origin: Instant, now: Instant) -> bool {
        now.saturating_duration_since(origin) <= self.maximum_age
    }

    fn freshness(&self, origin: Instant, now: Instant) -> Freshness {
        if self.recent(origin, now) {
            Freshness::Fresh
        } else {
            Freshness::Stale
        }
    }
}

#[cfg_attr(
    not(feature = "postgres"),
    allow(
        dead_code,
        reason = "only the control plane, behind `postgres`, publishes outside tests"
    )
)]
impl Writer {
    /// Publishes what one read returned, and says what that did.
    ///
    /// `at` is the instant the read vouches for. For a control-plane snapshot it
    /// is where age counts from. A snapshot read from the last-good file cannot
    /// vouch for itself, because the file carries no clock: its age counts from
    /// registration whatever `at` says, so a file never makes the process look
    /// fresher than it is.
    ///
    /// A snapshot that is [`Refused`](Disposition::Refused) changes only the
    /// recorded refusal: the snapshot served and its age are untouched.
    pub(crate) fn apply(&mut self, snapshot: Arc<Snapshot>, at: Instant) -> Disposition {
        let (next, disposition) = match self.published() {
            None => {
                let origin = match snapshot.provenance() {
                    Provenance::LastGoodFile => self.registered,
                    Provenance::ControlPlane => at,
                };
                (Published::first(snapshot, origin), Disposition::Applied)
            }
            Some(current) => current.next(snapshot, at),
        };
        self.install(next);
        disposition
    }

    /// Records that a read was structurally corrupt, so it never became a
    /// snapshot to judge. The refusal is carried by the record being served;
    /// before the first publication there is none to carry it, and the caller
    /// keeps the returned disposition.
    pub(crate) fn reject(&mut self, rejection: SnapshotRejection) -> Disposition {
        if let Some(current) = self.published() {
            self.install(current.rejected(rejection));
        }
        Disposition::Refused(Refusal::Rejected(rejection))
    }

    /// The record readers capture now.
    pub(crate) fn published(&self) -> Record {
        self.sender.borrow().clone()
    }

    fn install(&mut self, record: Published<Instant>) {
        // `send_replace` returns the replaced record once its lock is
        // released; it is dropped here, never under the lock.
        let replaced = self.sender.send_replace(Some(Arc::new(record)));
        drop(replaced);
    }
}

#[cfg(test)]
mod tests;
