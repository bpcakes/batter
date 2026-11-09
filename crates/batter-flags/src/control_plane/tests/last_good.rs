//! The last-good file through the registered loader: published before the
//! driver starts, served through an outage, and persisted after
//! control-plane publications.

use std::{path::PathBuf, time::Duration};

use batter_flags_core::{
    Freshness, Reason, Source, SwitchState, Toggle, Version, low_level::CandidateRow,
};

use super::super::{
    FileProblem, Gate, LastGoodStatus, LoadCause, LoadPolicy, PersistFailure, Served,
    SnapshotDirectory, SnapshotFilePath, StartupFile, source::fake::Script,
};
use super::support::*;
use crate::{
    last_good::format::{EntryValue, FileEntry, FileSnapshot, decode, encode},
    low_level::capture_now,
};

/// A fresh directory under the system temporary directory, removed on drop.
pub(super) struct Directory(pub(super) PathBuf);

impl Directory {
    pub(super) fn new() -> Self {
        let path =
            std::env::temp_dir().join(format!("batter-flags-last-good-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }

    pub(super) fn path(&self) -> SnapshotFilePath {
        SnapshotFilePath::new(&self.0, "last-good.bfsn").unwrap()
    }

    pub(super) fn open(&self) -> SnapshotDirectory {
        SnapshotDirectory::open(self.path()).unwrap()
    }

    pub(super) fn write(&self, file: &FileSnapshot) {
        std::fs::write(self.0.join("last-good.bfsn"), encode(file).unwrap()).unwrap();
    }

    pub(super) fn read(&self) -> Option<FileSnapshot> {
        std::fs::read(self.0.join("last-good.bfsn"))
            .ok()
            .map(|bytes| decode(&bytes).unwrap())
    }
}

impl Drop for Directory {
    fn drop(&mut self) {
        let _ =
            std::fs::set_permissions(&self.0, std::os::unix::fs::PermissionsExt::from_mode(0o700));
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A file at `version` with `halt` and `stale_halt` in the given states.
pub(super) fn file_of(
    handles: &Handles,
    version: u64,
    halt: &str,
    stale_halt: &str,
) -> FileSnapshot {
    let (input, _) = resolved(&[]);
    let (base, _) = input.into_parts();
    let entry = |key: &str, state: &str| FileEntry {
        key: key.into(),
        kind: "switch".into(),
        value_type: "switch".into(),
        fingerprint: *base
            .catalog()
            .declaration(key)
            .unwrap()
            .fingerprint()
            .as_bytes(),
        updated_version: version,
        value: crate::last_good::format::EntryValue::State(format!(
            "{{\"encoding\":1,\"switch\":\"{state}\"}}"
        )),
    };
    let _ = handles;
    FileSnapshot {
        scope: crate::last_good::scope_label(&base),
        catalog_digest: *base.catalog().digest().as_bytes(),
        manifest_digest: *base.catalog().manifest_digest().as_bytes(),
        version,
        entries: vec![entry("halt", halt), entry("stale_halt", stale_halt)],
    }
}

#[tokio::test(start_paused = true)]
async fn a_usable_file_is_published_before_the_driver_starts_and_served_through_an_outage() {
    let directory = Directory::new();
    let (input, handles) = resolved(&[]);
    directory.write(&file_of(&handles, 100, "engaged", "released"));
    let mut supervisor = supervisor();
    let policy = LoadPolicy::start_on_defaults(file_timing()).with_last_good(directory.open());
    let (flags, _) = register(
        &mut supervisor,
        input,
        policy,
        failing(LoadCause::Unavailable),
    )
    .unwrap();

    // Before the driver starts, every surface already sees the file.
    assert_eq!(
        flags.status().served,
        Served::PersistedFile {
            version: Version::new(100)
        }
    );
    let view = capture_now(&flags);
    assert!(
        matches!(view.source(), Source::PersistedFile { version } if version == Version::new(100))
    );
    let decision = view.switch(&handles.halt);
    assert_eq!(decision.value(), SwitchState::Engaged);
    assert_eq!(decision.reason(), Reason::Stale, "no clock confirms a file");
    let (_, published) = flags.readiness_condition(Gate::Published);
    let (_, fresh) = flags.readiness_condition(Gate::Fresh);
    assert!(published());
    assert!(!fresh());

    let running = supervisor.start();
    running.status().wait_ready().await.unwrap();
    assert_eq!(
        capture_now(&flags).switch(&handles.stale_halt).value(),
        SwitchState::Released
    );
    // The file's age counts from registration, so an engage-when-stale
    // switch engages only after the maximum age.
    tokio::time::sleep(FILE_MAXIMUM_AGE + SECOND).await;
    let view = capture_now(&flags);
    assert_eq!(view.freshness(), Freshness::Stale);
    assert_eq!(
        view.switch(&handles.stale_halt).value(),
        SwitchState::Engaged
    );
    assert_eq!(view.switch(&handles.halt).value(), SwitchState::Engaged);
    assert_eq!(
        flags.status().served,
        Served::PersistedFile {
            version: Version::new(100)
        }
    );
    let _ = running.shutdown().await;
}

#[tokio::test(start_paused = true)]
async fn a_damaged_or_foreign_file_publishes_nothing_and_says_why() {
    let directory = Directory::new();
    let (input, handles) = resolved(&[]);
    let mut foreign = file_of(&handles, 100, "engaged", "released");
    foreign.scope = "FLAG_:worker".into();
    directory.write(&foreign);
    let mut supervisor = supervisor();
    let policy = LoadPolicy::start_on_defaults(file_timing()).with_last_good(directory.open());
    let (flags, _) = register(
        &mut supervisor,
        input,
        policy,
        failing(LoadCause::Unavailable),
    )
    .unwrap();
    assert_eq!(
        flags.startup_file(),
        StartupFile::Unusable(FileProblem::Foreign)
    );
    assert!(matches!(flags.status().served, Served::DeclaredBase { .. }));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_first_publication_rewrites_a_newer_file_and_later_ones_skip_a_newer_file() {
    let directory = Directory::new();
    let (input, handles) = resolved(&[]);
    directory.write(&file_of(&handles, 100, "engaged", "released"));
    let mut supervisor = supervisor();
    let policy = LoadPolicy::start_on_defaults(file_timing()).with_last_good(directory.open());
    // A store restored to 90, below the file.
    let script = serve(&handles, 90, SwitchState::Released, Toggle::On);
    let (flags, fake) = register(&mut supervisor, input, policy, script).unwrap();
    let running = supervisor.start();
    running.status().wait_ready().await.unwrap();
    until(|| directory.read().is_some_and(|file| file.version == 90)).await;
    assert!(is_fresh(flags.status()), "{:?}", flags.status());
    assert_eq!(
        capture_now(&flags).switch(&handles.halt).value(),
        SwitchState::Released
    );

    // Another writer leaves a newer file; this process's next version skips it.
    directory.write(&file_of(&handles, 200, "engaged", "released"));
    fake.set(serve(&handles, 91, SwitchState::Released, Toggle::On));
    until(|| matches!(capture_now(&flags).source(), Source::ControlPlane { version, .. } if version == Version::new(91))).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(directory.read().unwrap().version, 200);
    assert_eq!(
        flags.last_good(),
        LastGoodStatus::UpToDate {
            written: Some(Version::new(90))
        },
        "the skipped publication is settled, not a failure"
    );
    let _ = running.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_held_switch_whose_state_came_from_the_file_is_written_without_it() {
    let directory = Directory::new();
    let (input, handles) = resolved(&[]);
    directory.write(&file_of(&handles, 100, "engaged", "released"));
    let mut supervisor = supervisor();
    let policy = LoadPolicy::start_on_defaults(file_timing()).with_last_good(directory.open());
    // The control plane's halt row is undecodable, so halt is held and keeps
    // the file's engaged state.
    let at = Version::new(101);
    let script = Script::Serve {
        version: at,
        rows: vec![
            CandidateRow::new(
                "halt",
                "switch",
                "switch",
                at,
                "{\"encoding\":2,\"switch\":\"engaged\"}",
            ),
            CandidateRow::flag(&handles.flag, Toggle::On, at),
        ],
    };
    let (flags, _) = register(&mut supervisor, input, policy, script).unwrap();
    let running = supervisor.start();
    running.status().wait_ready().await.unwrap();
    until(|| directory.read().is_some_and(|file| file.version == 101)).await;
    assert_eq!(
        capture_now(&flags).switch(&handles.halt).value(),
        SwitchState::Engaged
    );
    let entries: Vec<_> = directory
        .read()
        .unwrap()
        .entries
        .into_iter()
        .map(|entry| (entry.key, matches!(entry.value, EntryValue::State(_))))
        .collect();
    assert_eq!(
        entries,
        [("halt".to_owned(), false), ("fast_path".to_owned(), true)],
        "the file-sourced hold is written without its value"
    );
    let _ = running.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_persist_that_fails_never_withholds_readiness() {
    let directory = Directory::new();
    let (input, handles) = resolved(&[]);
    let opened = directory.open();
    std::fs::set_permissions(
        &directory.0,
        std::os::unix::fs::PermissionsExt::from_mode(0o500),
    )
    .unwrap();
    let mut supervisor = supervisor();
    let policy = LoadPolicy::unready_until_loaded(file_timing(), Duration::from_secs(60))
        .with_last_good(opened);
    let script = serve(&handles, 3, SwitchState::Released, Toggle::On);
    let (flags, _) = register(&mut supervisor, input, policy, script).unwrap();
    let running = supervisor.start();
    running.status().wait_ready().await.unwrap();
    until(|| {
        matches!(
            flags.last_good(),
            LastGoodStatus::Waiting {
                last_failure: Some(_),
                ..
            }
        )
    })
    .await;
    assert_eq!(
        flags.last_good(),
        LastGoodStatus::Waiting {
            version: Version::new(3),
            written: None,
            last_failure: Some(PersistFailure::Io)
        }
    );
    assert!(is_fresh(flags.status()), "{:?}", flags.status());
    let _ = running.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_held_directory_lock_is_busy_and_residue_is_removed() {
    let directory = Directory::new();
    let (input, handles) = resolved(&[]);
    std::fs::write(directory.0.join("last-good.bfsn.tmp"), b"residue").unwrap();
    let lock = std::fs::File::open(&directory.0).unwrap();
    lock.try_lock().unwrap();
    let mut supervisor = supervisor();
    let policy = LoadPolicy::start_on_defaults(file_timing()).with_last_good(directory.open());
    let (flags, fake) = register(
        &mut supervisor,
        input,
        policy,
        serve(&handles, 3, SwitchState::Released, Toggle::On),
    )
    .unwrap();
    let running = supervisor.start();
    running.status().wait_ready().await.unwrap();
    until(|| {
        matches!(
            flags.last_good(),
            LastGoodStatus::Waiting {
                last_failure: Some(_),
                ..
            }
        )
    })
    .await;
    assert!(matches!(
        flags.last_good(),
        LastGoodStatus::Waiting {
            last_failure: Some(PersistFailure::Busy),
            ..
        }
    ));
    assert!(directory.read().is_none());

    lock.unlock().unwrap();
    fake.set(serve(&handles, 4, SwitchState::Released, Toggle::On));
    until(|| directory.read().is_some_and(|file| file.version == 4)).await;
    assert!(
        !directory.0.join("last-good.bfsn.tmp").exists(),
        "the residue is gone"
    );
    let _ = running.shutdown().await;
}

/// Waits, for at most twenty seconds of real time, for `condition`.
pub(super) async fn until(condition: impl Fn() -> bool) {
    for _ in 0..200 {
        if condition() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("the condition never held");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_engagement_held_against_bad_tags_survives_a_restart() {
    let directory = Directory::new();
    let (input, handles) = resolved(&[]);
    let mut first = supervisor();
    let policy = LoadPolicy::start_on_defaults(file_timing()).with_last_good(directory.open());
    let script = serve(&handles, 1, SwitchState::Engaged, Toggle::Off);
    let (flags, fake) = register(&mut first, input, policy, script).unwrap();
    let running = first.start();
    running.status().wait_ready().await.unwrap();
    until(|| directory.read().is_some_and(|file| file.version == 1)).await;
    // Version 2 retags halt, so it is held, keeping its engaged state.
    let at = Version::new(2);
    fake.set(Script::Serve {
        version: at,
        rows: vec![CandidateRow::new(
            "halt",
            "release",
            "toggle",
            at,
            "{\"encoding\":1,\"variant\":\"on\"}",
        )],
    });
    until(|| directory.read().is_some_and(|file| file.version == 2)).await;
    assert_eq!(
        capture_now(&flags).switch(&handles.halt).value(),
        SwitchState::Engaged
    );
    let _ = running.shutdown().await;

    // A restart into an outage serves the engagement from the file.
    let (input, handles) = resolved(&[]);
    let mut restarted = supervisor();
    let policy = LoadPolicy::start_on_defaults(file_timing()).with_last_good(directory.open());
    let (flags, _) = register(
        &mut restarted,
        input,
        policy,
        failing(LoadCause::Unavailable),
    )
    .unwrap();
    assert_eq!(
        capture_now(&flags).switch(&handles.halt).value(),
        SwitchState::Engaged
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_waiting_persist_is_retried_through_an_outage() {
    let directory = Directory::new();
    let (input, handles) = resolved(&[]);
    let lock = std::fs::File::open(&directory.0).unwrap();
    lock.try_lock().unwrap();
    let mut supervisor = supervisor();
    let policy = LoadPolicy::start_on_defaults(file_timing()).with_last_good(directory.open());
    let (flags, fake) = register(
        &mut supervisor,
        input,
        policy,
        serve(&handles, 3, SwitchState::Engaged, Toggle::On),
    )
    .unwrap();
    let running = supervisor.start();
    running.status().wait_ready().await.unwrap();
    until(|| {
        matches!(
            flags.last_good(),
            LastGoodStatus::Waiting {
                last_failure: Some(PersistFailure::Busy),
                ..
            }
        )
    })
    .await;
    fake.set(failing(LoadCause::Unavailable));
    lock.unlock().unwrap();
    until(|| directory.read().is_some_and(|file| file.version == 3)).await;
    // The failure does not outlive the write that replaced it.
    until(|| {
        flags.last_good()
            == LastGoodStatus::UpToDate {
                written: Some(Version::new(3)),
            }
    })
    .await;
    let _ = running.shutdown().await;
}
