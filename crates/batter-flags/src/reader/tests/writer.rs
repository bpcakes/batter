//! The writer owns the publication protocol: what it refuses, what it records
//! and where it counts age from.
#![allow(clippy::unwrap_used, reason = "a panic fails the test")]
#![allow(clippy::missing_assert_message, reason = "test names locate failures")]

use batter_flags_core::{
    Toggle, Version,
    low_level::{Candidate, CandidateRow, Disposition, Refusal, SnapshotRejection},
};

use super::*;

#[test]
fn a_regressing_read_is_refused_and_renews_nothing() {
    let (base, f) = base();
    let scope = base.validation_scope();
    let registered = Instant::now();
    let (mut writer, reader) = Reader::live(base, PrePublication::Unready, MAX_AGE, registered);
    writer.apply(snapshot(&scope, &f, 8), registered);

    // A lower version cannot replace what is served, whoever reads it and
    // however it is presented: there is no way to publish it any other way.
    let refused = writer.apply(
        snapshot(&scope, &f, 4),
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
        writer.published().unwrap().last_refusal(),
        Some(refused_cause(refused))
    );
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
    writer.apply(snapshot(&scope, &f, 7), registered);

    // Version 7 again, read with a row the first read did not have.
    let at = Version::new(7);
    let drifted = Candidate::new(&scope)
        .at(at)
        .row(CandidateRow::flag(&f.flag, Toggle::On, at));
    let drifted = Arc::new(Snapshot::validate(&scope, drifted).unwrap());
    let disposition = writer.apply(drifted, registered + Duration::from_secs(60));
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
fn a_file_ages_from_registration_whatever_instant_it_is_applied_at() {
    let (base, f) = base();
    let scope = base.validation_scope();
    let registered = Instant::now();
    let (mut writer, reader) = Reader::live(base, PrePublication::Unready, MAX_AGE, registered);

    // Applied 250 s after registration, stamped with the read time. A file
    // carries no clock, so it cannot make the process look fresher than the
    // time since it registered.
    let read_at = registered + Duration::from_secs(250);
    writer.apply(file_snapshot(&scope, &f, 3), read_at);
    assert_eq!(
        reader.capture_at(registered + MAX_AGE).freshness(),
        Freshness::Fresh
    );
    let late = reader.capture_at(registered + MAX_AGE + Duration::from_nanos(1));
    assert_eq!(late.freshness(), Freshness::Stale);
    assert_eq!(
        reader
            .capture_at(read_at + Duration::from_secs(10))
            .freshness(),
        Freshness::Stale
    );
}

#[test]
fn the_first_control_plane_read_applies_even_below_the_files_version() {
    let (base, f) = base();
    let scope = base.validation_scope();
    let registered = Instant::now();
    let (mut writer, reader) = Reader::live(base, PrePublication::Unready, MAX_AGE, registered);
    writer.apply(file_snapshot(&scope, &f, 100), registered);

    // A file's version says nothing about the control plane's, and the
    // control-plane read ages from its own instant, not from registration.
    let read_at = registered + Duration::from_secs(80);
    assert_eq!(
        writer.apply(snapshot(&scope, &f, 5), read_at),
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
    writer.apply(snapshot(&scope, &f, 5), registered);

    // The file is read once, first. A later one cannot replace or renew.
    let disposition = writer.apply(
        file_snapshot(&scope, &f, 9),
        registered + Duration::from_secs(10),
    );
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

    // Before the first publication there is no record to carry the refusal.
    let disposition = writer.reject(SnapshotRejection::MissingVersion);
    assert_eq!(
        disposition,
        Disposition::Refused(Refusal::Rejected(SnapshotRejection::MissingVersion))
    );
    assert!(writer.published().is_none());
    assert!(matches!(
        reader.capture_at(registered).source(),
        Source::NotLoaded { .. }
    ));

    // After it, the record carries the refusal and serves what it served.
    writer.apply(snapshot(&scope, &f, 5), registered);
    writer.reject(SnapshotRejection::MissingVersion);
    assert_eq!(
        writer.published().unwrap().last_refusal(),
        Some(Refusal::Rejected(SnapshotRejection::MissingVersion))
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
        writer.apply(snapshot(&scope, &f, 5), registered + Duration::from_secs(1)),
        Disposition::Confirmed
    );
    assert_eq!(writer.published().unwrap().last_refusal(), None);
}

#[test]
fn the_writer_being_dropped_is_observed_and_what_was_published_keeps_serving() {
    let (base, f) = base();
    let scope = base.validation_scope();
    let registered = Instant::now();
    let (mut writer, reader) = Reader::live(base, PrePublication::Unready, MAX_AGE, registered);
    assert!(!reader.writer_gone());
    writer.apply(snapshot(&scope, &f, 2), registered);
    assert!(!reader.writer_gone());

    drop(writer);
    assert!(reader.writer_gone());
    let view = reader.capture_at(registered);
    assert_eq!(view.evaluate(&f.flag).value(), Toggle::On);
}
