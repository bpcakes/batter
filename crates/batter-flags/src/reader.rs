//! The reader: how registered flags turn into one view per unit of work.
//!
//! A view is a frozen evaluation context: the base, at most one snapshot, a
//! source and the freshness its publisher computed. The reader is the
//! publisher. In declared mode it has nothing to publish; for injected test
//! flags it holds one fixed snapshot; behind a control plane it follows one
//! immutable [`Record`], replaced whole by the single writer's [`Publisher`],
//! so the evidence that decides freshness is always the evidence of the
//! snapshot being served.
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

#[cfg(any(test, feature = "test-support"))]
use batter_flags_core::low_level::Snapshot;
use batter_flags_core::{
    FlagKey, Freshness, PrePublication, SnapshotView,
    low_level::{Base, Candidate, Disposition, HoldCause, Publisher, Record, ViewError},
};
use tokio::{sync::watch, time::Instant};

/// What the control plane's publisher serves now.
type Current = Arc<Record<Instant>>;

/// Where a reader's views come from.
enum Mode {
    /// Declared mode: the settings read at startup, no control plane.
    Declared,
    /// Injected test flags: one fixed snapshot and freshness, only in builds
    /// with `test-support`.
    #[cfg(any(test, feature = "test-support"))]
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
    record: LiveRecord,
    pre_publication: PrePublication,
}

/// The control plane's side of a live reader: the record it serves, whether
/// its writer remains and how old a confirmation may be. Only a live reader
/// has one, so no other mode answers these questions with a stand-in.
#[derive(Clone)]
pub(crate) struct LiveRecord {
    receiver: watch::Receiver<Current>,
    maximum_age: Duration,
}

/// The shared half of the reader: what every clone of the flags captures from.
pub(crate) struct Reader {
    base: Base,
    mode: Mode,
}

/// The only way to publish: held by the component that loads snapshots, and
/// dropped when it ends.
///
/// It owns core's [`Publisher`], which owns the publication protocol: what
/// readers serve is the publisher's [`Record`], and every read goes through
/// [`Publisher::judge`], so core's rules apply to every read: a version never
/// goes backwards, a repeated version with other content is drift, a held
/// kill switch keeps the state it served, and a file is only ever restored,
/// first. Nothing can publish a record the publisher did not derive. The
/// mutating methods take `&mut self`, so there is one writer at a time.
#[cfg_attr(
    not(feature = "postgres"),
    allow(
        dead_code,
        reason = "only the control plane, behind `postgres`, publishes outside tests"
    )
)]
pub(crate) struct Writer {
    sender: watch::Sender<Current>,
    publisher: Publisher<Instant>,
}

/// What ungated work may run on.
#[cfg_attr(
    not(feature = "runledger"),
    allow(
        dead_code,
        reason = "only the job surface, behind `runledger`, defers ungated work"
    )
)]
pub(crate) enum WorkCapture {
    /// A view to run on.
    Serve(SnapshotView),
    /// Nothing is published and the load policy withholds readiness.
    Unready,
}

/// The run's deadline passed while its read was judged, so nothing was
/// published or recorded.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Late;

impl Reader {
    /// A reader for declared mode.
    pub(crate) fn declared(base: Base) -> Self {
        Self {
            base,
            mode: Mode::Declared,
        }
    }

    /// A reader of one fixed snapshot, as the fixture path serves it.
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn test(base: Base, snapshot: Arc<Snapshot>, freshness: Freshness) -> Self {
        Self {
            base,
            mode: Mode::Test {
                snapshot,
                freshness,
            },
        }
    }

    /// A reader over a control plane that has published nothing, and the
    /// writer that publishes to it.
    ///
    /// `registered` is where age counts from until the first publication,
    /// and `maximum_age` is how long a confirmation keeps a view fresh.
    #[cfg(test)]
    pub(crate) fn live(
        base: Base,
        pre_publication: PrePublication,
        maximum_age: Duration,
        registered: Instant,
    ) -> (Writer, Self) {
        let publisher = Publisher::new(&base.validation_scope(), registered);
        let (writer, reader, _) = Self::publishing(base, pre_publication, maximum_age, publisher);
        (writer, reader)
    }

    /// A reader over a control plane whose publisher already exists, as one
    /// restored from the last-good file, the writer that owns it and the
    /// control plane's side of the reader.
    #[cfg_attr(
        not(feature = "postgres"),
        allow(
            dead_code,
            reason = "only the control plane, behind `postgres`, publishes outside tests"
        )
    )]
    pub(crate) fn publishing(
        base: Base,
        pre_publication: PrePublication,
        maximum_age: Duration,
        publisher: Publisher<Instant>,
    ) -> (Writer, Self, LiveRecord) {
        let (sender, receiver) = watch::channel(Arc::new(publisher.record().clone()));
        let record = LiveRecord {
            receiver,
            maximum_age,
        };
        let reader = Self {
            base,
            mode: Mode::Live(Live {
                record: record.clone(),
                pre_publication,
            }),
        };
        (Writer { sender, publisher }, reader, record)
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

    /// One view for work that is not gated by a switch, unless the load
    /// policy withholds readiness and nothing is published: such work has no
    /// state to run on, and would otherwise run on the declared defaults the
    /// process is not yet ready to serve. Decided from the same record the
    /// view would be built from.
    #[cfg_attr(
        not(feature = "runledger"),
        allow(
            dead_code,
            reason = "only the job surface, behind `runledger`, defers ungated work"
        )
    )]
    pub(crate) fn capture_for_work(&self) -> WorkCapture {
        match &self.mode {
            Mode::Live(live) => {
                let record = live.record.current();
                match (&*record, live.pre_publication) {
                    (Record::Unpublished(_), PrePublication::Unready) => WorkCapture::Unready,
                    (Record::Unpublished(_), PrePublication::ServeDefaults)
                    | (Record::Published(_), _) => {
                        let now = Instant::now();
                        let view = self
                            .base
                            .record_view(&record, live.pre_publication, |origin| {
                                live.record.recent(*origin, now)
                            })
                            .unwrap_or_else(|_: ViewError| {
                                self.base
                                    .not_loaded_view(PrePublication::Unready, Freshness::Stale)
                            });
                        WorkCapture::Serve(view)
                    }
                }
            }
            Mode::Declared => WorkCapture::Serve(self.capture()),
            #[cfg(any(test, feature = "test-support"))]
            Mode::Test { .. } => WorkCapture::Serve(self.capture()),
        }
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
            #[cfg(any(test, feature = "test-support"))]
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
}

impl Live {
    fn capture(&self, base: &Base, now: Instant) -> Result<SnapshotView, ViewError> {
        // Clone the `Arc`; the read lock is released at the end of the
        // statement, before any evaluation.
        let record = self.record.current();
        base.record_view(&record, self.pre_publication, |origin| {
            self.record.recent(*origin, now)
        })
    }
}

#[cfg_attr(
    not(feature = "postgres"),
    allow(
        dead_code,
        reason = "only the control plane, behind `postgres`, publishes outside tests"
    )
)]
impl LiveRecord {
    /// The record being served.
    pub(crate) fn current(&self) -> Current {
        self.receiver.borrow().clone()
    }

    /// Whether the writer has been dropped, so nothing will ever publish
    /// again.
    pub(crate) fn writer_gone(&self) -> bool {
        self.receiver.has_changed().is_err()
    }

    /// The freshness a view of `record` captured at `now` has.
    pub(crate) fn freshness(&self, record: &Record<Instant>, now: Instant) -> Freshness {
        record.freshness(|origin| self.recent(*origin, now))
    }

    /// The held keys of the record being served, as a view captured at
    /// `now` holds them, with why each is held; empty before anything is
    /// published. A stale view can hold more than a fresh one.
    pub(crate) fn held_at(&self, now: Instant) -> Vec<(FlagKey, HoldCause)> {
        self.held_in(&self.current(), now)
    }

    /// The held keys of `record`, as a view of it captured at `now` holds
    /// them, so a report built from one record pairs its version with its
    /// own holds.
    pub(crate) fn held_in(
        &self,
        record: &Record<Instant>,
        now: Instant,
    ) -> Vec<(FlagKey, HoldCause)> {
        let Some(snapshot) = record.snapshot() else {
            return Vec::new();
        };
        snapshot
            .held(self.freshness(record, now))
            .map(|(key, cause)| (key.clone(), cause))
            .collect()
    }

    fn recent(&self, origin: Instant, now: Instant) -> bool {
        now.saturating_duration_since(origin) <= self.maximum_age
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
    /// Judges one read and publishes it unless the run's `deadline` passed
    /// while it was judged, and says what that did.
    ///
    /// The read vouches for the instant it is published at. Core's publisher
    /// counts a file's age from registration and refuses a file offered as a
    /// read. A snapshot that is [`Refused`](Disposition::Refused) changes only
    /// the recorded refusal: the snapshot served and its age are untouched.
    ///
    /// # Errors
    ///
    /// [`Late`] when the deadline passed while the read was judged: nothing is
    /// published or recorded.
    pub(crate) fn read(
        &mut self,
        candidate: Candidate,
        deadline: Instant,
    ) -> Result<Disposition, Late> {
        let judged = self.publisher.judge(candidate);
        let at = Instant::now();
        if at >= deadline {
            return Err(Late);
        }
        let disposition = judged.publish(at);
        self.install();
        Ok(disposition)
    }

    /// Judges and publishes one read that vouches for `at`, with no deadline.
    #[cfg(test)]
    pub(crate) fn read_at(&mut self, candidate: Candidate, at: Instant) -> Disposition {
        let disposition = self.publisher.judge(candidate).publish(at);
        self.install();
        disposition
    }

    /// The record readers capture now.
    #[cfg(test)]
    pub(crate) fn published(&self) -> &Record<Instant> {
        self.publisher.record()
    }

    fn install(&mut self) {
        // `send_replace` returns the replaced record once its lock is
        // released; it is dropped here, never under the lock.
        let replaced = self
            .sender
            .send_replace(Arc::new(self.publisher.record().clone()));
        drop(replaced);
    }
}

#[cfg(test)]
mod tests;
