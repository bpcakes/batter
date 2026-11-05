//! The published record: one immutable value replaced whole on every poll, so a
//! snapshot is never judged against another snapshot's evidence.
#![allow(clippy::unwrap_used, reason = "a panic fails the test")]
#![allow(clippy::missing_assert_message, reason = "test names locate failures")]

#[path = "support/fixtures.rs"]
mod fixtures;
#[path = "support/world.rs"]
mod world;

use std::sync::Arc;

use batter_flags_core::{
    Freshness, Source, Toggle,
    low_level::{Disposition, Published, Refusal, SnapshotRejection, Standing, ViewError},
};
use world::{base, file_snapshot, flag_row, snapshot, version, world};

const MAX_AGE: u64 = 60;

/// The loader's judgement: `now`, in ticks, is within the maximum age of the
/// record's evidence.
fn recent(now: u64) -> impl Fn(&u64) -> bool {
    move |origin| now - origin <= MAX_AGE
}

#[test]
fn a_record_crosses_threads() {
    fn shareable<T: Send + Sync + 'static>() {}
    shareable::<Published<u64>>();
}

#[test]
fn a_file_ages_from_registration_and_confirms_nothing() {
    let (catalog, h) = world();
    let base = base(catalog, &h.serving, &[]);
    let restored = file_snapshot(&base, 100, [flag_row(&h.fast_path, Toggle::On, 99)]);
    let first = Published::first(restored, 10);

    assert_eq!(
        first.authority().standing(),
        Standing::Persisted {
            version: version(100)
        }
    );
    assert_eq!((*first.age_origin(), first.last_refusal()), (10, None));
    let early = base.published_view(&first, recent(50)).unwrap();
    assert_eq!(early.freshness(), Freshness::Fresh);
    assert_eq!(
        early.source(),
        Source::PersistedFile {
            version: version(100)
        }
    );
    // Age runs from registration, so an old file goes stale however fresh its rows.
    let late = base.published_view(&first, recent(200)).unwrap();
    assert_eq!(late.freshness(), Freshness::Stale);
}

#[test]
fn the_first_control_plane_read_replaces_the_file_at_any_version_and_renews_age() {
    let (catalog, h) = world();
    let base = base(catalog, &h.serving, &[]);
    let rows = || [flag_row(&h.fast_path, Toggle::On, 4)];
    let first = Published::first(file_snapshot(&base, 100, rows()), 10);

    let read = snapshot(&base, 5, rows());
    let (applied, disposition) = first.next(Arc::clone(&read), 90);
    assert_eq!(disposition, Disposition::Applied);
    assert!(Arc::ptr_eq(applied.snapshot(), &read));
    assert_eq!(*applied.age_origin(), 90, "the read's own evidence");
    assert!(matches!(
        applied.authority().standing(),
        Standing::ControlPlane { version: applied_at, digest }
            if applied_at == version(5) && digest == read.digest()
    ));
    let view = base.published_view(&applied, recent(120)).unwrap();
    assert_eq!(view.freshness(), Freshness::Fresh);
    assert!(matches!(view.source(), Source::ControlPlane { version: at, .. } if at == version(5)));
    // The record that was replaced is untouched.
    assert_eq!(*first.age_origin(), 10);
}

#[test]
fn a_confirming_read_renews_age_and_keeps_the_snapshot_it_confirmed() {
    let (catalog, h) = world();
    let base = base(catalog, &h.serving, &[]);
    let rows = || [flag_row(&h.fast_path, Toggle::On, 4)];
    let serving = Published::first(snapshot(&base, 5, rows()), 100);

    let (confirmed, disposition) = serving.next(snapshot(&base, 5, rows()), 150);
    assert_eq!(disposition, Disposition::Confirmed);
    assert!(Arc::ptr_eq(serving.snapshot(), confirmed.snapshot()));
    assert_eq!(*confirmed.age_origin(), 150);
    assert_eq!(*serving.age_origin(), 100);
}

#[test]
fn a_refused_read_changes_nothing_but_the_recorded_refusal() {
    let (catalog, h) = world();
    let base = base(catalog, &h.serving, &[]);
    let rows = |on: Toggle| [flag_row(&h.fast_path, on, 4)];
    let serving = Published::first(snapshot(&base, 5, rows(Toggle::On)), 100);

    let refusals = [
        (
            serving.next(snapshot(&base, 4, rows(Toggle::On)), 200),
            Refusal::Regressed {
                applied: version(5),
                observed: version(4),
            },
        ),
        (
            serving.next(snapshot(&base, 5, rows(Toggle::Off)), 200),
            Refusal::DigestDrift {
                version: version(5),
            },
        ),
        (
            serving.next(file_snapshot(&base, 9, rows(Toggle::On)), 200),
            Refusal::FileAfterPublication,
        ),
    ];
    for ((record, disposition), refusal) in refusals {
        assert_eq!(disposition, Disposition::Refused(refusal));
        assert_eq!(record.last_refusal(), Some(refusal));
        assert!(Arc::ptr_eq(record.snapshot(), serving.snapshot()));
        assert_eq!(
            *record.age_origin(),
            100,
            "nothing was confirmed: {refusal:?}"
        );
        assert_eq!(record.authority(), serving.authority());
    }
    let rejected = serving.rejected(SnapshotRejection::MissingVersion);
    assert_eq!(
        rejected.last_refusal(),
        Some(Refusal::Rejected(SnapshotRejection::MissingVersion))
    );
    assert_eq!(*rejected.age_origin(), 100);
    assert_eq!(
        serving.last_refusal(),
        None,
        "the record that was read is unchanged"
    );
}

/// A refusal stays until a read applies or confirms.
#[test]
fn a_refusal_clears_only_through_a_publication_or_a_confirming_read() {
    let (catalog, h) = world();
    let base = base(catalog, &h.serving, &[]);
    let rows = |on: Toggle| [flag_row(&h.fast_path, on, 4)];
    let serving = Published::first(snapshot(&base, 5, rows(Toggle::On)), 100);
    let (refused, _) = serving.next(snapshot(&base, 4, rows(Toggle::On)), 110);

    // Another refusal replaces the recorded one.
    let (again, _) = refused.next(snapshot(&base, 5, rows(Toggle::Off)), 120);
    assert_eq!(
        again.last_refusal(),
        Some(Refusal::DigestDrift {
            version: version(5)
        })
    );
    // Reading nothing at all leaves it.
    assert!(refused.last_refusal().is_some());

    let (confirmed, _) = refused.next(snapshot(&base, 5, rows(Toggle::On)), 130);
    assert_eq!(confirmed.last_refusal(), None);
    let (applied, _) = refused.next(snapshot(&base, 6, rows(Toggle::Off)), 130);
    assert_eq!(applied.last_refusal(), None);
}

/// The reason for the record: the evidence that decides freshness is the
/// evidence of the snapshot being served, and cannot be taken from another.
#[test]
fn a_view_judges_the_evidence_of_the_record_it_was_built_from() {
    let (catalog, h) = world();
    let base = base(catalog, &h.serving, &[]);
    let early = Published::first(
        snapshot(&base, 5, [flag_row(&h.fast_path, Toggle::On, 4)]),
        100,
    );
    let late = Published::first(
        snapshot(&base, 6, [flag_row(&h.fast_path, Toggle::Off, 5)]),
        200,
    );

    // The same question, "was the evidence taken at tick 100?", of two records.
    let was_at_100 = |origin: &u64| *origin == 100;
    let first = base.published_view(&early, was_at_100).unwrap();
    let second = base.published_view(&late, was_at_100).unwrap();
    assert_eq!(first.freshness(), Freshness::Fresh);
    assert_eq!(second.freshness(), Freshness::Stale);
    assert_eq!(first.evaluate(&h.fast_path).value(), Toggle::On);
    assert_eq!(second.evaluate(&h.fast_path).value(), Toggle::Off);
}

#[test]
fn a_record_of_another_base_is_refused() {
    let (catalog, h) = world();
    let base = base(catalog, &h.serving, &[]);
    let record = Published::first(snapshot(&base, 5, []), 100);
    let (other_catalog, other) = world();
    let other_base = self::base(other_catalog, &other.serving, &[]);
    assert_eq!(
        other_base.published_view(&record, recent(100)).unwrap_err(),
        ViewError::ForeignSnapshot
    );
}
