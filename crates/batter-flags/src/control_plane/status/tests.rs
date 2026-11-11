//! A status pairs a publication with the verdict of the read that made it:
//! the refresh publishes and records its verdict in one critical section, and
//! a status reads under that same lock.
#![allow(
    clippy::unwrap_used,
    clippy::missing_assert_message,
    reason = "failure-contract assertions"
)]

use std::sync::mpsc;

use batter_flags_core::{
    CatalogBuilder, PrePublication, SettingsPrefix, Toggle,
    low_level::{BaseBuilder, Publisher},
};

use super::*;

const MAXIMUM_AGE: Duration = Duration::from_secs(60);

fn deadline() -> Instant {
    Instant::now() + MAXIMUM_AGE
}

/// The version the live record serves, if anything is published.
fn served(shared: &Shared) -> Option<Version> {
    match &*shared.live().current() {
        Record::Unpublished(_) => None,
        Record::Published(published) => Some(published.snapshot().version()),
    }
}

/// While the shared state's lock is held, a concurrent publication cannot
/// become visible: the writer judges, publishes and records its verdict inside
/// that lock, so a status can never pair the new publication with the
/// previous read's refusal. Were the writer run outside the lock, the record
/// would change here while the test still holds it.
#[test]
fn a_publication_becomes_visible_only_together_with_its_own_verdict() {
    let (mut catalog, serving, _) =
        CatalogBuilder::new(SettingsPrefix::new("FLAG_").unwrap(), "serving", "hosted").unwrap();
    catalog
        .mode_flag("fast_path", Toggle::Off)
        .declare()
        .unwrap();
    let base = BaseBuilder::new(catalog.build(), serving).unwrap().finish();
    let scope = base.validation_scope();
    let publisher = Publisher::new(&scope, Instant::now());
    let (mut writer, reader, live) =
        Reader::publishing(base, PrePublication::Unready, MAXIMUM_AGE, publisher);
    let shared = Arc::new(Shared::new(Arc::new(reader), live));

    let candidate = |version: u64| Candidate::new(&scope, Version::new(version));
    let applied = shared.publish(&mut writer, candidate(2), deadline(), Version::new(2));
    assert_eq!(applied.1, Ok(()));
    // A regressing read is refused and leaves version 2 served.
    let refused = shared.publish(&mut writer, candidate(1), deadline(), Version::new(1));
    assert_eq!(refused.1, Err(LoadCause::Regressed));
    let before = shared.status(Instant::now());
    assert_eq!(served(&shared), Some(Version::new(2)));
    assert!(matches!(
        before.latest_read,
        LatestRead::Refused {
            refusal: Refusal::Regressed { applied, .. },
            ..
        } if applied == Version::new(2)
    ));

    let held = shared.state();
    let (started, reached) = mpsc::channel();
    let publishing = std::thread::spawn({
        let shared = shared.clone();
        let next = candidate(3);
        move || {
            started.send(()).unwrap();
            shared.publish(&mut writer, next, deadline(), Version::new(3))
        }
    });
    reached.recv().unwrap();
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(
        served(&shared),
        Some(Version::new(2)),
        "the publication stayed invisible while its verdict could not be recorded"
    );
    drop(held);

    assert_eq!(publishing.join().unwrap().1, Ok(()));
    let after = shared.status(Instant::now());
    assert!(matches!(
        after.served,
        Served::ControlPlane { version, .. } if version == Version::new(3)
    ));
    assert_eq!(after.latest_read, LatestRead::Loaded);
}
