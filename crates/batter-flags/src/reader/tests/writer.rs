//! The writer owns the publication protocol: what it refuses, what it records
//! and where it counts age from.
#![allow(clippy::unwrap_used, reason = "a panic fails the test")]
#![allow(clippy::missing_assert_message, reason = "test names locate failures")]

use batter_flags_core::{
    Toggle, Version,
    low_level::{Candidate, CandidateRow, Disposition, Refusal, SnapshotRejection},
};

/// A read judged when its deadline has already passed publishes nothing and
/// records nothing.
#[test]
fn a_read_judged_past_its_deadline_changes_nothing() {
    let (base, f) = base();
    let scope = base.validation_scope();
    let registered = Instant::now();
    let (mut writer, reader) = Reader::live(base, PrePublication::Unready, MAX_AGE, registered);
    assert_eq!(
        writer.read(candidate(&scope, &f, 2), Instant::now()),
        Err(Late)
    );
    assert!(matches!(writer.published(), Record::Unpublished(_)));
    assert_eq!(writer.published().last_refusal(), None);

    writer.read_at(candidate(&scope, &f, 2), registered);
    let deadline = Instant::now();
    assert_eq!(writer.read(duplicated(&scope, &f), deadline), Err(Late));
    assert_eq!(
        writer.published().last_refusal(),
        None,
        "no refusal recorded"
    );
    assert!(matches!(
        reader.capture_at(registered).source(),
        Source::ControlPlane { version, .. } if version == Version::new(2)
    ));
    assert_eq!(
        writer.read(candidate(&scope, &f, 4), Instant::now() + MAX_AGE),
        Ok(Disposition::Applied)
    );
}

use super::*;

#[test]
fn a_regressing_read_is_refused_and_renews_nothing() {
    let (base, f) = base();
    let scope = base.validation_scope();
    let registered = Instant::now();
    let (mut writer, reader) = Reader::live(base, PrePublication::Unready, MAX_AGE, registered);
    writer.read_at(candidate(&scope, &f, 8), registered);

    // A lower version cannot replace what is served, whoever reads it and
    // however it is presented: there is no way to publish it any other way.
    let refused = writer.read_at(
        candidate(&scope, &f, 4),
        registered + Duration::from_secs(60),
    );
    assert_eq!(
        refused,
        Disposition::Refused(Refusal::Regressed {
            applied: Version::new(8),
            observed: Version::new(4)
        })
    );
    let view = reader.capture_at(registered + MAX_AGE + Duration::from_secs(1));
    assert_eq!(
        view.freshness(),
        Freshness::Stale,
        "the refused read renewed nothing"
    );
    assert_eq!(
        view.source(),
        Source::ControlPlane {
            version: Version::new(8),
            freshness: Freshness::Stale
        }
    );
    assert_eq!(view.evaluate(&f.flag).value(), Toggle::On);
    assert_eq!(
        writer.published().last_refusal(),
        Some(refused_cause(refused))
    );
}

/// A read at version 9 that repeats one key: structurally corrupt.
fn duplicated(scope: &ValidationScope, f: &Fixture) -> Candidate {
    let at = Version::new(9);
    Candidate::new(scope, at)
        .row(CandidateRow::flag(&f.flag, Toggle::On, at))
        .row(CandidateRow::flag(&f.flag, Toggle::On, at))
}

/// The refusal a disposition carries.
fn refused_cause(disposition: Disposition) -> Refusal {
    match disposition {
        Disposition::Refused(refusal) => refusal,
        other => panic!("expected a refusal, found {other:?}"),
    }
}

#[test]
fn the_same_version_with_other_content_is_drift_and_confirms_nothing() {
    let (base, f) = base();
    let scope = base.validation_scope();
    let registered = Instant::now();
    let (mut writer, reader) = Reader::live(base, PrePublication::Unready, MAX_AGE, registered);
    writer.read_at(candidate(&scope, &f, 7), registered);

    // Version 7 again, read with a row the first read did not have.
    let at = Version::new(7);
    let drifted = Candidate::new(&scope, at).row(CandidateRow::flag(&f.flag, Toggle::On, at));
    let disposition = writer.read_at(drifted, registered + Duration::from_secs(60));
    assert_eq!(
        disposition,
        Disposition::Refused(Refusal::DigestDrift { version: at })
    );
    // The snapshot served is the first read, and its age was not renewed.
    let view = reader.capture_at(registered + MAX_AGE + Duration::from_secs(1));
    assert_eq!(view.freshness(), Freshness::Stale);
    assert_eq!(view.evaluate(&f.flag).value(), Toggle::Off);
}

#[test]
fn a_restored_file_ages_from_registration() {
    let (base, f) = base();
    let registered = Instant::now();
    let (_writer, reader) = restored(base, &f, 3, registered);

    // A file carries no clock, so it cannot make the process look fresher
    // than the time since it registered.
    assert_eq!(
        reader.capture_at(registered + MAX_AGE).freshness(),
        Freshness::Fresh
    );
    let late = reader.capture_at(registered + MAX_AGE + Duration::from_nanos(1));
    assert_eq!(late.freshness(), Freshness::Stale);
}

/// A file is only ever restored. Offered as a read, even before anything is
/// published, it is refused, so it can never be stamped with a read time.
#[test]
fn a_file_offered_as_a_read_is_refused() {
    let (base, f) = base();
    let scope = base.validation_scope();
    let registered = Instant::now();
    let (mut writer, reader) = Reader::live(base, PrePublication::Unready, MAX_AGE, registered);
    let at = Version::new(3);
    let file = Candidate::from_last_good_file(&scope, at).row(CandidateRow::switch(
        &f.halt,
        SwitchState::Engaged,
        at,
    ));
    assert_eq!(
        writer.read_at(file, registered + Duration::from_secs(250)),
        Disposition::Refused(Refusal::FileAfterPublication)
    );
    assert!(matches!(
        reader.capture_at(registered).source(),
        Source::NotLoaded { .. }
    ));
}

#[test]
fn the_first_control_plane_read_applies_even_below_the_files_version() {
    let (base, f) = base();
    let scope = base.validation_scope();
    let registered = Instant::now();
    let (mut writer, reader) = restored(base, &f, 100, registered);

    // A file's version says nothing about the control plane's, and the
    // control-plane read ages from its own instant, not from registration.
    let read_at = registered + Duration::from_secs(80);
    assert_eq!(
        writer.read_at(candidate(&scope, &f, 5), read_at),
        Disposition::Applied
    );
    let view = reader.capture_at(read_at + MAX_AGE);
    assert_eq!(
        view.source(),
        Source::ControlPlane {
            version: Version::new(5),
            freshness: Freshness::Fresh
        }
    );
}

#[test]
fn a_file_after_a_publication_is_refused() {
    let (base, f) = base();
    let scope = base.validation_scope();
    let registered = Instant::now();
    let (mut writer, reader) = Reader::live(base, PrePublication::Unready, MAX_AGE, registered);
    writer.read_at(candidate(&scope, &f, 5), registered);

    // The file is read once, first. A later one cannot replace or renew.
    let at = Version::new(9);
    let file = Candidate::from_last_good_file(&scope, at).row(CandidateRow::switch(
        &f.halt,
        SwitchState::Engaged,
        at,
    ));
    let disposition = writer.read_at(file, registered + Duration::from_secs(10));
    assert_eq!(
        disposition,
        Disposition::Refused(Refusal::FileAfterPublication)
    );
    assert_eq!(
        reader.capture_at(registered).source(),
        Source::ControlPlane {
            version: Version::new(5),
            freshness: Freshness::Fresh
        }
    );
}

#[test]
fn a_structurally_corrupt_read_is_recorded_without_touching_what_is_served() {
    let (base, f) = base();
    let scope = base.validation_scope();
    let registered = Instant::now();
    let (mut writer, reader) = Reader::live(base, PrePublication::Unready, MAX_AGE, registered);

    // Before the first publication the record carries the refusal too.
    let disposition = writer.read_at(duplicated(&scope, &f), registered);
    assert_eq!(
        disposition,
        Disposition::Refused(Refusal::Rejected(SnapshotRejection::DuplicateKey {
            row: 1
        }))
    );
    assert!(matches!(writer.published(), Record::Unpublished(_)));
    assert_eq!(
        writer.published().last_refusal(),
        Some(Refusal::Rejected(SnapshotRejection::DuplicateKey {
            row: 1
        }))
    );
    assert!(matches!(
        reader.capture_at(registered).source(),
        Source::NotLoaded { .. }
    ));

    // After it, the record carries the refusal and serves what it served.
    writer.read_at(candidate(&scope, &f, 5), registered);
    writer.read_at(duplicated(&scope, &f), registered);
    assert_eq!(
        writer.published().last_refusal(),
        Some(Refusal::Rejected(SnapshotRejection::DuplicateKey {
            row: 1
        }))
    );
    assert_eq!(
        reader.capture_at(registered).source(),
        Source::ControlPlane {
            version: Version::new(5),
            freshness: Freshness::Fresh
        }
    );
    // A read that confirms what is served clears it.
    assert_eq!(
        writer.read_at(
            candidate(&scope, &f, 5),
            registered + Duration::from_secs(1)
        ),
        Disposition::Confirmed
    );
    assert_eq!(writer.published().last_refusal(), None);
}

#[test]
fn the_writer_being_dropped_is_observed_and_what_was_published_keeps_serving() {
    let (base, f) = base();
    let scope = base.validation_scope();
    let registered = Instant::now();
    let publisher = Publisher::new(&scope, registered);
    let (mut writer, reader, live) =
        Reader::publishing(base, PrePublication::Unready, MAX_AGE, publisher);
    assert!(!live.writer_gone());
    writer.read_at(candidate(&scope, &f, 2), registered);
    assert!(!live.writer_gone());

    drop(writer);
    assert!(live.writer_gone());
    let view = reader.capture_at(registered);
    assert_eq!(view.evaluate(&f.flag).value(), Toggle::On);
}
