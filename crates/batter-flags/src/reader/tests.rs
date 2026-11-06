//! What no public surface can reach: publication, age, writer loss and
//! concurrent capture.
#![allow(clippy::unwrap_used, reason = "a panic fails the test")]
#![allow(clippy::missing_assert_message, reason = "test names locate failures")]

use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

use batter_flags_core::{
    CatalogBuilder, CivilDate, Expiry, KillSwitch, ReleaseFlag, SafeState, SettingsPrefix, Source,
    StaleRule, SwitchState, Toggle, Version,
    low_level::{BaseBuilder, Candidate, CandidateRow, ValidationScope},
};

use super::*;

const MAX_AGE: Duration = Duration::from_secs(100);

struct Fixture {
    flag: ReleaseFlag<Toggle>,
    halt: KillSwitch,
    stale_halt: KillSwitch,
}

fn base() -> (Base, Fixture) {
    let mut builder = CatalogBuilder::new(SettingsPrefix::new("FLAG_").unwrap());
    let serving = builder.process_kind("serving").unwrap();
    builder.environment_class("hosted").unwrap();
    let expiry = Expiry::on(CivilDate::new(2027, 3, 31).unwrap());
    let flag = builder
        .release_flag("fast_path", Toggle::Off, expiry)
        .declare()
        .unwrap();
    let halt = builder
        .kill_switch("halt", SafeState::Released, StaleRule::HoldLastKnown)
        .declare()
        .unwrap();
    let stale_halt = builder
        .kill_switch(
            "stale_halt",
            SafeState::Released,
            StaleRule::EngageWhenStale,
        )
        .declare()
        .unwrap();
    let base = BaseBuilder::new(builder.build().unwrap(), serving)
        .unwrap()
        .finish();
    (
        base,
        Fixture {
            flag,
            halt,
            stale_halt,
        },
    )
}

/// What a snapshot at `version` says: the flag is on for even versions and the
/// switch is engaged for odd ones, so a view's content names its version.
fn content(version: u64) -> (Toggle, SwitchState) {
    if version.is_multiple_of(2) {
        (Toggle::On, SwitchState::Released)
    } else {
        (Toggle::Off, SwitchState::Engaged)
    }
}

fn candidate(scope: &ValidationScope, fixture: &Fixture, version: u64) -> Candidate {
    let at = Version::new(version);
    let (flag, halt) = content(version);
    Candidate::new(scope)
        .at(at)
        .row(CandidateRow::flag(&fixture.flag, flag, at))
        .row(CandidateRow::switch(&fixture.halt, halt, at))
}

fn snapshot(scope: &ValidationScope, fixture: &Fixture, version: u64) -> Arc<Snapshot> {
    Arc::new(Snapshot::validate(scope, candidate(scope, fixture, version)).unwrap())
}

#[test]
fn before_a_publication_the_view_is_not_loaded_and_ages_from_registration() {
    let (base, f) = base();
    let registered = Instant::now();
    let (_writer, reader) = Reader::live(base, PrePublication::ServeDefaults, MAX_AGE, registered);

    let fresh = reader.capture_at(registered + MAX_AGE);
    assert!(matches!(fresh.source(), Source::NotLoaded { .. }));
    assert_eq!(fresh.freshness(), Freshness::Fresh);
    let stale = reader.capture_at(registered + MAX_AGE + Duration::from_nanos(1));
    assert_eq!(stale.freshness(), Freshness::Stale);
    // A process that has waited longer than the maximum age applies each
    // switch's stale rule, publication or not.
    assert_eq!(fresh.switch(&f.stale_halt).value(), SwitchState::Released);
    assert_eq!(stale.switch(&f.stale_halt).value(), SwitchState::Engaged);
}

#[test]
fn the_load_policy_decides_what_a_view_before_publication_answers() {
    let (base, f) = base();
    let (_writer, defaults) =
        Reader::live(base, PrePublication::ServeDefaults, MAX_AGE, Instant::now());
    let decision = defaults.capture().evaluate(&f.flag);
    assert_eq!(decision.reason(), batter_flags_core::Reason::Default);

    let (base, f) = self::base();
    let (_writer, unready) = Reader::live(base, PrePublication::Unready, MAX_AGE, Instant::now());
    let decision = unready.capture().evaluate(&f.flag);
    assert_eq!(decision.reason(), batter_flags_core::Reason::Error);
}

#[test]
fn a_published_record_is_judged_by_the_evidence_it_carries() {
    let (base, f) = base();
    let scope = base.validation_scope();
    let registered = Instant::now();
    let (mut writer, reader) = Reader::live(base, PrePublication::Unready, MAX_AGE, registered);

    let confirmed_at = registered + Duration::from_secs(40);
    assert_eq!(
        writer.apply(snapshot(&scope, &f, 6), confirmed_at),
        Disposition::Applied
    );
    let view = reader.capture_at(confirmed_at + MAX_AGE);
    assert_eq!(
        view.source(),
        Source::ControlPlane {
            version: Version::new(6),
            freshness: Freshness::Fresh
        }
    );
    assert_eq!(view.evaluate(&f.flag).value(), Toggle::On);
    let late = reader.capture_at(confirmed_at + MAX_AGE + Duration::from_nanos(1));
    assert_eq!(late.freshness(), Freshness::Stale);

    // A later read that confirms the same snapshot renews its age and leaves
    // what is served untouched.
    let reconfirmed_at = confirmed_at + Duration::from_secs(90);
    assert_eq!(
        writer.apply(snapshot(&scope, &f, 6), reconfirmed_at),
        Disposition::Confirmed
    );
    let renewed = reader.capture_at(confirmed_at + Duration::from_secs(150));
    assert_eq!(renewed.freshness(), Freshness::Fresh);
    assert_eq!(renewed.evaluate(&f.flag).value(), Toggle::On);
}

#[test]
fn a_file_publication_is_served_as_a_persisted_file() {
    let (base, f) = base();
    let scope = base.validation_scope();
    let registered = Instant::now();
    let (mut writer, reader) = Reader::live(base, PrePublication::Unready, MAX_AGE, registered);
    let file = file_snapshot(&scope, &f, 3);
    assert_eq!(writer.apply(file, registered), Disposition::Applied);
    let view = reader.capture_at(registered);
    assert_eq!(
        view.source(),
        Source::PersistedFile {
            version: Version::new(3)
        }
    );
    assert_eq!(view.switch(&f.halt).value(), SwitchState::Engaged);
}

fn file_snapshot(scope: &ValidationScope, fixture: &Fixture, version: u64) -> Arc<Snapshot> {
    let at = Version::new(version);
    let file = Candidate::from_last_good_file(scope)
        .at(at)
        .row(CandidateRow::switch(
            &fixture.halt,
            SwitchState::Engaged,
            at,
        ));
    Arc::new(Snapshot::validate(scope, file).unwrap())
}

#[test]
fn a_reader_without_a_control_plane_has_no_writer_to_lose() {
    let (base, _f) = base();
    assert!(!Reader::declared(base).writer_gone());
}

#[tokio::test(start_paused = true)]
async fn capture_follows_the_runtimes_clock() {
    let (base, f) = base();
    let scope = base.validation_scope();
    let (mut writer, reader) = Reader::live(base, PrePublication::Unready, MAX_AGE, Instant::now());
    writer.apply(snapshot(&scope, &f, 2), Instant::now());
    assert_eq!(reader.capture().freshness(), Freshness::Fresh);
    tokio::time::advance(MAX_AGE).await;
    assert_eq!(reader.capture().freshness(), Freshness::Fresh);
    tokio::time::advance(Duration::from_secs(1)).await;
    assert_eq!(reader.capture().freshness(), Freshness::Stale);
}

#[test]
fn a_foreign_snapshot_makes_a_loud_view_not_a_silent_default() {
    // By construction a reader only holds snapshots of its own base. If it
    // ever held another's, every decision would report an error.
    let (base, f) = base();
    let (other, other_fixture) = self::base();
    let scope = other.validation_scope();
    let foreign = snapshot(&scope, &other_fixture, 2);
    let reader = Reader::test(base, foreign, Freshness::Fresh);
    let view = reader.capture();
    assert!(matches!(view.source(), Source::NotLoaded { .. }));
    assert_eq!(view.freshness(), Freshness::Stale);
    assert_eq!(
        view.evaluate(&f.flag).reason(),
        batter_flags_core::Reason::Error
    );
}

/// Every view a concurrent capture returns is one publication whole: the
/// version it reports, the content it serves and the freshness of the
/// evidence that came with that snapshot all belong together.
#[test]
fn under_a_concurrent_writer_every_view_matches_its_own_publication() {
    const VERSIONS: u64 = 400;
    const READERS: usize = 4;
    // Version `v` is confirmed at `origin + v` seconds and views are judged at
    // `origin + 250`, so it is fresh exactly when `250 - v <= 100`, and every
    // later version is confirmed in the future, which is fresh too.
    let judged = Duration::from_secs(250);
    let fresh_from = 150;

    let (base, f) = base();
    let scope = base.validation_scope();
    let origin = Instant::now();
    let (mut writer, reader) = Reader::live(base, PrePublication::ServeDefaults, MAX_AGE, origin);
    let reader = Arc::new(reader);
    let snapshots: Vec<Arc<Snapshot>> = (1..=VERSIONS)
        .map(|version| snapshot(&scope, &f, version))
        .collect();

    let done = AtomicBool::new(false);
    let loaded = AtomicU64::new(0);
    let distinct = AtomicU64::new(0);
    std::thread::scope(|threads| {
        for _ in 0..READERS {
            threads.spawn(|| {
                let mut previous = 0;
                let mut finished = false;
                while !finished {
                    // Read the flag before capturing, so a last capture follows
                    // the writer's last publication.
                    finished = done.load(Ordering::Acquire);
                    let view = reader.capture_at(origin + judged);
                    let Source::ControlPlane { version, freshness } = view.source() else {
                        continue;
                    };
                    let version = version.get();
                    let (flag, halt) = content(version);
                    assert_eq!(view.evaluate(&f.flag).value(), flag, "version {version}");
                    assert_eq!(view.switch(&f.halt).value(), halt, "version {version}");
                    let expected = if version >= fresh_from {
                        Freshness::Fresh
                    } else {
                        Freshness::Stale
                    };
                    assert_eq!(freshness, expected, "version {version}");
                    assert!(version >= previous, "a reader never goes backwards");
                    if version != previous {
                        distinct.fetch_add(1, Ordering::Relaxed);
                    }
                    previous = version;
                    loaded.fetch_add(1, Ordering::Relaxed);
                }
                assert_eq!(
                    previous, VERSIONS,
                    "the last capture sees the last publication"
                );
            });
        }
        for (index, snapshot) in snapshots.iter().enumerate() {
            let at = origin + Duration::from_secs(index as u64 + 1);
            let disposition = writer.apply(Arc::clone(snapshot), at);
            assert_eq!(disposition, Disposition::Applied);
        }
        done.store(true, Ordering::Release);
    });
    assert!(loaded.load(Ordering::Relaxed) >= READERS as u64);
    assert!(distinct.load(Ordering::Relaxed) >= READERS as u64);
}

mod writer;
