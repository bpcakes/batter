//! The publisher and its record: one immutable value replaced whole on every
//! read, advanced only by the publisher, so a snapshot is never judged against
//! another snapshot's evidence and no step of a publication is the loader's.
#![allow(clippy::unwrap_used, reason = "a panic fails the test")]
#![allow(clippy::missing_assert_message, reason = "test names locate failures")]

#[path = "support/fixtures.rs"]
mod fixtures;
#[path = "support/world.rs"]
mod world;

use std::sync::Arc;

use batter_flags_core::{
    Freshness, Layer, PrePublication, Source, SwitchState, Toggle,
    low_level::{
        Base, Candidate, CandidateRow, Disposition, Publisher, Record, Refusal, Restored,
        SnapshotRejection, Standing, ViewError,
    },
};
use world::{base, flag_row, snapshot, switch_row, undecodable_row, version, world};

const MAX_AGE: u64 = 60;

/// The loader's judgement: `now`, in ticks, is within the maximum age of the
/// record's evidence.
fn recent(now: u64) -> impl Fn(&u64) -> bool {
    move |origin| now - origin <= MAX_AGE
}

/// A control-plane read of `rows` at `at` for `base`.
fn read(base: &Base, at: u64, rows: impl IntoIterator<Item = CandidateRow>) -> Candidate {
    Candidate::new(&base.validation_scope(), version(at)).rows(rows)
}

/// A publisher that published `rows` at version 5 with evidence at tick 100.
fn serving(base: &Base, rows: impl IntoIterator<Item = CandidateRow>) -> Publisher<u64> {
    let mut publisher = Publisher::new(&base.validation_scope(), 0);
    assert_eq!(
        publisher.judge(read(base, 5, rows)).publish(100),
        Disposition::Applied
    );
    publisher
}

fn published(publisher: &Publisher<u64>) -> &batter_flags_core::low_level::Published<u64> {
    match publisher.record() {
        Record::Published(published) => published,
        Record::Unpublished(_) => panic!("nothing is published"),
    }
}

#[test]
fn a_publisher_and_its_record_cross_threads() {
    fn shareable<T: Send + Sync + 'static>() {}
    shareable::<Publisher<u64>>();
    shareable::<Record<u64>>();
}

#[test]
fn before_any_publication_views_are_not_loaded_and_age_from_registration() {
    let (catalog, h) = world();
    let base = base(catalog, &h.serving, &[]);
    let publisher = Publisher::new(&base.validation_scope(), 10);
    let record = publisher.record();
    assert!(matches!(record, Record::Unpublished(_)));
    assert_eq!(record.standing(), Standing::Unpublished);
    assert_eq!((*record.age_origin(), record.last_refusal()), (10, None));
    assert!(record.snapshot().is_none());

    let early = base
        .record_view(record, PrePublication::ServeDefaults, recent(50))
        .unwrap();
    assert_eq!(early.freshness(), Freshness::Fresh);
    assert!(matches!(early.source(), Source::NotLoaded { .. }));
    let late = base
        .record_view(record, PrePublication::ServeDefaults, recent(200))
        .unwrap();
    assert_eq!(late.freshness(), Freshness::Stale);
}

#[test]
fn a_restored_file_ages_from_registration_and_confirms_nothing() {
    let (catalog, h) = world();
    let base = base(catalog, &h.serving, &[]);
    let (publisher, restored) = Publisher::restore(
        &base.validation_scope(),
        10,
        version(100),
        [flag_row(&h.fast_path, Toggle::On, 99)],
    );
    assert_eq!(restored, Restored::Published);
    let first = published(&publisher);
    assert_eq!(
        first.standing(),
        Standing::Persisted {
            version: version(100)
        }
    );
    assert_eq!((*first.age_origin(), first.last_refusal()), (10, None));
    let early = base
        .record_view(publisher.record(), PrePublication::Unready, recent(50))
        .unwrap();
    assert_eq!(early.freshness(), Freshness::Fresh);
    assert_eq!(
        early.source(),
        Source::PersistedFile {
            version: version(100)
        }
    );
    // Age runs from registration, so an old file goes stale however fresh its rows.
    let late = base.published_view(first, recent(200)).unwrap();
    assert_eq!(late.freshness(), Freshness::Stale);
}

#[test]
fn a_corrupt_file_publishes_nothing_and_its_refusal_is_recorded() {
    let (catalog, h) = world();
    let base = base(catalog, &h.serving, &[]);
    let row = flag_row(&h.fast_path, Toggle::On, 99);
    let (publisher, restored) = Publisher::restore(
        &base.validation_scope(),
        10,
        version(100),
        [row.clone(), row],
    );
    let rejection = SnapshotRejection::DuplicateKey { row: 1 };
    assert_eq!(restored, Restored::Rejected(rejection));
    let record = publisher.record();
    assert!(matches!(record, Record::Unpublished(_)));
    assert_eq!(record.last_refusal(), Some(Refusal::Rejected(rejection)));
    assert_eq!(*record.age_origin(), 10);
}

#[test]
fn the_first_control_plane_read_replaces_the_file_at_any_version_and_renews_age() {
    let (catalog, h) = world();
    let base = base(catalog, &h.serving, &[]);
    let rows = || [flag_row(&h.fast_path, Toggle::On, 4)];
    let (mut publisher, _) = Publisher::restore(&base.validation_scope(), 10, version(100), rows());
    let file = publisher.record().clone();

    assert_eq!(
        publisher.judge(read(&base, 5, rows())).publish(90),
        Disposition::Applied
    );
    let applied = published(&publisher);
    assert_eq!(*applied.age_origin(), 90, "the read's own evidence");
    assert!(matches!(
        applied.standing(),
        Standing::ControlPlane { version: applied_at, digest }
            if applied_at == version(5) && digest == applied.snapshot().digest()
    ));
    let view = base.published_view(applied, recent(120)).unwrap();
    assert_eq!(view.freshness(), Freshness::Fresh);
    assert!(matches!(view.source(), Source::ControlPlane { version: at, .. } if at == version(5)));
    // A record handed out earlier is untouched.
    assert_eq!(*file.age_origin(), 10);
}

#[test]
fn a_confirming_read_renews_age_and_keeps_the_snapshot_it_confirmed() {
    let (catalog, h) = world();
    let base = base(catalog, &h.serving, &[]);
    let rows = || [flag_row(&h.fast_path, Toggle::On, 4)];
    let mut publisher = serving(&base, rows());
    let before = Arc::clone(published(&publisher).snapshot());

    assert_eq!(
        publisher.judge(read(&base, 5, rows())).publish(150),
        Disposition::Confirmed
    );
    let confirmed = published(&publisher);
    assert!(Arc::ptr_eq(&before, confirmed.snapshot()));
    assert_eq!(*confirmed.age_origin(), 150);
}

#[test]
fn a_refused_read_changes_nothing_but_the_recorded_refusal() {
    let (catalog, h) = world();
    let base = base(catalog, &h.serving, &[]);
    let rows = |on: Toggle| [flag_row(&h.fast_path, on, 4)];
    let scope = base.validation_scope();
    let refusals = [
        (
            read(&base, 4, rows(Toggle::On)),
            Refusal::Regressed {
                applied: version(5),
                observed: version(4),
            },
        ),
        (
            read(&base, 5, rows(Toggle::Off)),
            Refusal::DigestDrift {
                version: version(5),
            },
        ),
        (
            Candidate::from_last_good_file(&scope, version(9)).rows(rows(Toggle::On)),
            Refusal::FileAfterPublication,
        ),
        (
            Candidate::new(&scope, version(6))
                .rows(rows(Toggle::On))
                .rows(rows(Toggle::On)),
            Refusal::Rejected(SnapshotRejection::DuplicateKey { row: 1 }),
        ),
    ];
    for (candidate, refusal) in refusals {
        let mut publisher = serving(&base, rows(Toggle::On));
        let before = published(&publisher).clone();
        assert_eq!(
            publisher.judge(candidate).publish(200),
            Disposition::Refused(refusal)
        );
        let record = published(&publisher);
        assert_eq!(record.last_refusal(), Some(refusal));
        assert!(Arc::ptr_eq(record.snapshot(), before.snapshot()));
        assert_eq!(
            *record.age_origin(),
            100,
            "nothing was confirmed: {refusal:?}"
        );
        assert_eq!(record.standing(), before.standing());
        assert_eq!(
            before.last_refusal(),
            None,
            "a record handed out is unchanged"
        );
    }
}

/// A version below the applied one is a regression whatever its rows hold.
#[test]
fn a_regressed_read_is_refused_before_its_rows_are_judged() {
    let (catalog, h) = world();
    let base = base(catalog, &h.serving, &[]);
    let mut publisher = serving(&base, [flag_row(&h.fast_path, Toggle::On, 4)]);
    let row = flag_row(&h.fast_path, Toggle::Off, 3);
    assert_eq!(
        publisher
            .judge(read(&base, 4, [row.clone(), row]))
            .publish(200),
        Disposition::Refused(Refusal::Regressed {
            applied: version(5),
            observed: version(4),
        })
    );
}

/// A refusal stays until a read applies or confirms.
#[test]
fn a_refusal_clears_only_through_a_publication_or_a_confirming_read() {
    let (catalog, h) = world();
    let base = base(catalog, &h.serving, &[]);
    let rows = |on: Toggle| [flag_row(&h.fast_path, on, 4)];
    let mut publisher = serving(&base, rows(Toggle::On));
    let _ = publisher
        .judge(read(&base, 4, rows(Toggle::On)))
        .publish(110);

    // Another refusal replaces the recorded one.
    let _ = publisher
        .judge(read(&base, 5, rows(Toggle::Off)))
        .publish(120);
    assert_eq!(
        publisher.record().last_refusal(),
        Some(Refusal::DigestDrift {
            version: version(5)
        })
    );
    let _ = publisher
        .judge(read(&base, 5, rows(Toggle::On)))
        .publish(130);
    assert_eq!(publisher.record().last_refusal(), None, "confirmed");

    let _ = publisher
        .judge(read(&base, 4, rows(Toggle::On)))
        .publish(140);
    assert!(publisher.record().last_refusal().is_some());
    let _ = publisher
        .judge(read(&base, 6, rows(Toggle::Off)))
        .publish(150);
    assert_eq!(publisher.record().last_refusal(), None, "applied");
}

/// A refusal of the first read used to be dropped, because a record could
/// not exist before something was published.
#[test]
fn a_refusal_before_the_first_publication_is_recorded() {
    let (catalog, h) = world();
    let base = base(catalog, &h.serving, &[]);
    let mut publisher = Publisher::new(&base.validation_scope(), 10);
    let row = || flag_row(&h.fast_path, Toggle::On, 1);
    let corrupt = read(&base, 1, [row(), row()]);
    let refusal = Refusal::Rejected(SnapshotRejection::DuplicateKey { row: 1 });
    assert_eq!(
        publisher.judge(corrupt).publish(20),
        Disposition::Refused(refusal)
    );
    let record = publisher.record();
    assert!(matches!(record, Record::Unpublished(_)));
    assert_eq!(record.last_refusal(), Some(refusal));
    assert_eq!(*record.age_origin(), 10, "registration is still the origin");

    assert_eq!(
        publisher
            .judge(read(&base, 2, [flag_row(&h.fast_path, Toggle::On, 2)]))
            .publish(30),
        Disposition::Applied
    );
    assert_eq!(publisher.record().last_refusal(), None);
}

/// A file is only ever restored: offered as a read, even first, it is refused.
#[test]
fn a_file_offered_as_a_read_is_refused_even_before_anything_is_published() {
    let (catalog, h) = world();
    let base = base(catalog, &h.serving, &[]);
    let scope = base.validation_scope();
    let mut publisher = Publisher::new(&scope, 10);
    let file = Candidate::from_last_good_file(&scope, version(100)).row(flag_row(
        &h.fast_path,
        Toggle::On,
        99,
    ));
    assert_eq!(
        publisher.judge(file).publish(20),
        Disposition::Refused(Refusal::FileAfterPublication)
    );
    assert!(matches!(publisher.record(), Record::Unpublished(_)));
}

/// A second file offered right after a restore is refused, and the restored
/// record keeps its snapshot, standing and age origin.
#[test]
fn a_second_file_after_a_restore_is_refused_and_changes_nothing() {
    let (catalog, h) = world();
    let base = base(catalog, &h.serving, &[]);
    let scope = base.validation_scope();
    let (mut publisher, restored) = Publisher::restore(
        &scope,
        10,
        version(100),
        [flag_row(&h.fast_path, Toggle::On, 99)],
    );
    assert_eq!(restored, Restored::Published);
    let before = published(&publisher).clone();
    let second = Candidate::from_last_good_file(&scope, version(200)).row(flag_row(
        &h.fast_path,
        Toggle::Off,
        199,
    ));
    assert_eq!(
        publisher.judge(second).publish(20),
        Disposition::Refused(Refusal::FileAfterPublication)
    );
    let after = published(&publisher);
    assert!(Arc::ptr_eq(after.snapshot(), before.snapshot()));
    assert_eq!(
        after.standing(),
        Standing::Persisted {
            version: version(100)
        }
    );
    assert_eq!(*after.age_origin(), 10);
    assert_eq!(after.last_refusal(), Some(Refusal::FileAfterPublication));
}

/// A loader whose deadline passes while a read is judged drops the judgement.
#[test]
fn a_judged_read_that_is_not_published_changes_nothing() {
    let (catalog, h) = world();
    let base = base(catalog, &h.serving, &[]);
    let mut publisher = Publisher::new(&base.validation_scope(), 10);
    drop(publisher.judge(read(&base, 2, [flag_row(&h.fast_path, Toggle::On, 2)])));
    assert!(matches!(publisher.record(), Record::Unpublished(_)));

    let mut publisher = serving(&base, [flag_row(&h.fast_path, Toggle::On, 4)]);
    drop(publisher.judge(read(&base, 4, [])));
    drop(publisher.judge(read(&base, 6, [flag_row(&h.fast_path, Toggle::Off, 6)])));
    let record = published(&publisher);
    assert_eq!(record.snapshot().version(), version(5));
    assert_eq!((*record.age_origin(), record.last_refusal()), (100, None));
}

/// Holding never releases an engaged switch: the publisher carries what it
/// serves into each read, whatever the candidate carried.
#[test]
fn the_publisher_carries_held_switch_states_into_every_read() {
    let (catalog, h) = world();
    let base = base(catalog, &h.serving, &[]);
    let scope = base.validation_scope();
    let mut publisher = serving(
        &base,
        [switch_row(&h.checkout_halt, SwitchState::Engaged, 3)],
    );
    // A candidate that carried another, released snapshot.
    let released = snapshot(
        &base,
        4,
        [switch_row(&h.checkout_halt, SwitchState::Released, 4)],
    );
    let held = read(
        &base,
        6,
        [undecodable_row("checkout_halt", "switch", "switch", 6)],
    )
    .carrying(&released)
    .unwrap();
    assert_eq!(publisher.judge(held).publish(130), Disposition::Applied);
    let view = base
        .record_view(publisher.record(), PrePublication::Unready, recent(140))
        .unwrap();
    let switch = view.switch(&h.checkout_halt);
    assert_eq!(
        switch.value(),
        SwitchState::Engaged,
        "holding never releases"
    );
    assert_eq!(switch.layer(), &Layer::Held { since: version(5) });

    // A second held read keeps the original `since`.
    let again = read(
        &base,
        7,
        [undecodable_row("checkout_halt", "switch", "switch", 7)],
    );
    assert_eq!(publisher.judge(again).publish(150), Disposition::Applied);
    let view = base
        .record_view(publisher.record(), PrePublication::Unready, recent(160))
        .unwrap();
    assert_eq!(
        view.switch(&h.checkout_halt).layer(),
        &Layer::Held { since: version(5) }
    );

    // A first publication carries nothing, whatever its candidate carried.
    let engaged = snapshot(
        &base,
        3,
        [switch_row(&h.checkout_halt, SwitchState::Engaged, 3)],
    );
    let mut fresh = Publisher::new(&scope, 0);
    let first = read(
        &base,
        6,
        [undecodable_row("checkout_halt", "switch", "switch", 6)],
    )
    .carrying(&engaged)
    .unwrap();
    assert_eq!(fresh.judge(first).publish(10), Disposition::Applied);
    let view = base
        .record_view(fresh.record(), PrePublication::Unready, recent(20))
        .unwrap();
    assert_eq!(
        view.switch(&h.checkout_halt).value(),
        SwitchState::Released,
        "nothing was published before, so the safe state serves"
    );
}

/// The reason for the record: the evidence that decides freshness is the
/// evidence of the snapshot being served, and cannot be taken from another.
#[test]
fn a_view_judges_the_evidence_of_the_record_it_was_built_from() {
    let (catalog, h) = world();
    let base = base(catalog, &h.serving, &[]);
    let early = serving(&base, [flag_row(&h.fast_path, Toggle::On, 4)]);
    let mut late = Publisher::new(&base.validation_scope(), 0);
    let _ = late
        .judge(read(&base, 6, [flag_row(&h.fast_path, Toggle::Off, 5)]))
        .publish(200);

    // The same question, "was the evidence taken at tick 100?", of two records.
    let was_at_100 = |origin: &u64| *origin == 100;
    let first = base
        .record_view(early.record(), PrePublication::Unready, was_at_100)
        .unwrap();
    let second = base
        .record_view(late.record(), PrePublication::Unready, was_at_100)
        .unwrap();
    assert_eq!(first.freshness(), Freshness::Fresh);
    assert_eq!(second.freshness(), Freshness::Stale);
    assert_eq!(first.evaluate(&h.fast_path).value(), Toggle::On);
    assert_eq!(second.evaluate(&h.fast_path).value(), Toggle::Off);
}

#[test]
fn a_record_or_candidate_of_another_base_is_refused() {
    let (catalog, h) = world();
    let base = base(catalog, &h.serving, &[]);
    let mut publisher = serving(&base, []);
    let (other_catalog, other) = world();
    let other_base = self::base(other_catalog, &other.serving, &[]);
    assert_eq!(
        other_base
            .record_view(publisher.record(), PrePublication::Unready, recent(100))
            .unwrap_err(),
        ViewError::ForeignSnapshot
    );
    assert_eq!(
        publisher.judge(read(&other_base, 6, [])).publish(110),
        Disposition::Refused(Refusal::Rejected(SnapshotRejection::ForeignScope))
    );
    assert_eq!(published(&publisher).snapshot().version(), version(5));
}
