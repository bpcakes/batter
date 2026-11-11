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
