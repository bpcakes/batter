//! What every protected surface and the directory check see of the
//! last-good file before the first control-plane attempt.

use batter_flags_core::{Source, SwitchState, Version};

use super::super::{
    DirectoryError, LoadCause, LoadPolicy, SnapshotDirectory, SnapshotFilePath,
    source::fake::Script,
};
use super::{
    last_good::{Directory, file_of},
    support::*,
};

#[tokio::test(start_paused = true)]
async fn a_file_publication_is_read_by_its_state_not_as_awaiting_publication() {
    use batter_flags_core::low_level::GateReading;
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
    let reading = |switch| {
        crate::flags::gate::SwitchGate::single(&flags, switch)
            .unwrap()
            .enter()
            .reading
    };
    assert!(matches!(reading(&handles.halt), GateReading::Tripped(_)));
    assert!(matches!(reading(&handles.stale_halt), GateReading::Open(_)));
}

#[test]
fn an_unusable_directory_fails_startup_and_names_are_reserved() {
    let directory = Directory::new();
    assert_eq!(
        SnapshotFilePath::new("relative", "last-good.bfsn").unwrap_err(),
        DirectoryError::NotAbsolute
    );
    for name in [
        "snapshot",
        ".bfsn",
        "a.bfsn.tmp",
        "last-good.bfsn.probe",
        "last-good.bfsn.probe-1-2-3",
        "spa ce.bfsn",
    ] {
        assert_eq!(
            SnapshotFilePath::new(&directory.0, name).unwrap_err(),
            DirectoryError::InvalidName,
            "{name}"
        );
    }
    let missing = SnapshotFilePath::new(directory.0.join("absent"), "last-good.bfsn").unwrap();
    assert_eq!(
        SnapshotDirectory::open(missing).unwrap_err(),
        DirectoryError::Missing
    );
    std::fs::set_permissions(
        &directory.0,
        std::os::unix::fs::PermissionsExt::from_mode(0o500),
    )
    .unwrap();
    assert_eq!(
        SnapshotDirectory::open(directory.path()).unwrap_err(),
        DirectoryError::NotWritable
    );
    std::fs::set_permissions(
        &directory.0,
        std::os::unix::fs::PermissionsExt::from_mode(0o700),
    )
    .unwrap();
    // A FIFO in the file's place is not read, so it never blocks startup.
    let status = std::process::Command::new("mkfifo")
        .arg(directory.0.join("last-good.bfsn"))
        .status()
        .unwrap();
    assert!(status.success());
    let opened = SnapshotDirectory::open(directory.path()).unwrap();
    assert!(matches!(
        opened.initial(),
        crate::last_good::Initial::Unreadable
    ));
    assert!(!format!("{opened:?}").contains("BFSN"));
}

#[test]
fn a_leftover_probe_never_blocks_a_later_open_and_concurrent_opens_succeed() {
    let directory = Directory::new();
    // What a process that died between creating and removing its probe left.
    let leftovers = [
        "last-good.bfsn.probe".to_owned(),
        format!("last-good.bfsn.probe-{}-0-0", std::process::id()),
    ];
    for leftover in &leftovers {
        std::fs::write(directory.0.join(leftover), b"").unwrap();
    }
    let _first = directory.open();
    let openers: Vec<_> = (0..8)
        .map(|_| {
            let path = directory.path();
            std::thread::spawn(move || SnapshotDirectory::open(path).map(drop))
        })
        .collect();
    for opener in openers {
        assert_eq!(opener.join().unwrap(), Ok(()));
    }
    let mut left: Vec<_> = std::fs::read_dir(&directory.0)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    left.sort();
    assert_eq!(left, leftovers, "every probe an open made was removed");
}

#[test]
fn reading_a_fifo_returns_at_once_without_a_writer() {
    let directory = Directory::new();
    let fifo = directory.0.join("last-good.bfsn");
    let status = std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .unwrap();
    assert!(status.success());
    // The descriptor is opened before its type is judged, so the open itself
    // must not wait for a writer that never comes.
    let (sent, received) = std::sync::mpsc::channel();
    std::thread::spawn(move || sent.send(crate::last_good::read_bounded(&fifo)));
    let initial = received
        .recv_timeout(std::time::Duration::from_secs(10))
        .expect("the read waited on the FIFO");
    assert!(matches!(initial, crate::last_good::Initial::Unreadable));
}

#[tokio::test(start_paused = true)]
async fn a_gated_periodic_run_before_the_first_attempt_sees_the_file() {
    use std::sync::{Arc, Mutex};
    let directory = Directory::new();
    let (input, handles) = resolved(&[]);
    directory.write(&file_of(&handles, 100, "engaged", "released"));
    let mut supervisor = supervisor();
    let policy = LoadPolicy::start_on_defaults(file_timing()).with_last_good(directory.open());
    // The first control-plane read never returns within its run.
    let (flags, _) = register(&mut supervisor, input, policy, Script::Hang).unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let record = seen.clone();
    let halt = handles.halt.clone();
    flags
        .gate_periodic_in(
            &mut supervisor,
            "storage.maintenance",
            crate::periodic::GatedPeriodicPolicy::new(SECOND * 30, SECOND).unwrap(),
            &handles.stale_halt,
            move |_, view| {
                record
                    .lock()
                    .unwrap()
                    .push((view.source(), view.switch(&halt).value()));
                std::future::ready(Ok::<
                    (),
                    batter_core::periodic::PeriodicFailure<std::io::Error>,
                >(()))
            },
        )
        .unwrap();
    let running = supervisor.start();
    running.status().wait_ready().await.unwrap();
    let seen = seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 1);
    assert!(matches!(seen[0].0, Source::PersistedFile { version } if version == Version::new(100)));
    assert_eq!(seen[0].1, SwitchState::Engaged);
    let _ = running.shutdown().await;
}

#[cfg(feature = "runledger")]
#[tokio::test(start_paused = true)]
async fn a_job_invocation_before_the_first_attempt_sees_the_file() {
    use std::{
        sync::{Arc, Mutex},
        time::Duration,
    };

    use batter_flags_core::KillSwitch;
    use batter_runledger::test_support::{HandlerExit, HandlerInvocation};
    use runledger_core::{
        jobs::{JobCompletion, JobExecution, JobFailure, JobType},
        prelude::async_trait,
    };

    use crate::jobs::{FlagAwareHandler, JobFlags};

    struct Probe(Arc<Mutex<Vec<(Source, SwitchState)>>>, KillSwitch);
    #[async_trait]
    impl FlagAwareHandler for Probe {
        fn job_type(&self) -> JobType<'static> {
            JobType::new("test.probe")
        }
        async fn execute(
            &self,
            _: JobExecution<'_>,
            flags: &JobFlags,
            _: serde_json::Value,
        ) -> Result<JobCompletion, JobFailure> {
            let decision = flags.switch(&self.1);
            self.0
                .lock()
                .unwrap()
                .push((decision.source(), decision.value()));
            Ok(JobCompletion::success())
        }
    }

    let directory = Directory::new();
    let (input, handles) = resolved(&[]);
    directory.write(&file_of(&handles, 100, "engaged", "released"));
    let mut supervisor = supervisor();
    let policy = LoadPolicy::start_on_defaults(file_timing()).with_last_good(directory.open());
    let (flags, _) = register(&mut supervisor, input, policy, Script::Hang).unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let handler = flags.flagged(
        Probe(seen.clone(), handles.halt.clone()),
        crate::jobs::Recheck::after(Duration::from_secs(30)).unwrap(),
    );
    // No driver has started, so no control-plane read has been attempted.
    let result = HandlerInvocation::new(Duration::from_secs(5))
        .unwrap()
        .execute(&handler, serde_json::Value::Null)
        .await;
    assert!(matches!(result.exit, HandlerExit::Returned(Ok(_))));
    let seen = seen.lock().unwrap().clone();
    assert!(matches!(seen[0].0, Source::PersistedFile { version } if version == Version::new(100)));
    assert_eq!(seen[0].1, SwitchState::Engaged);
}
