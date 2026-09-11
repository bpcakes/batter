use crate::load;
use crate::temp_dir;
use batter::{
    cleanup::{CleanupBudget, CleanupOutcome},
    lifecycle::{Readiness, ShutdownBudget},
    operation::OperationContext,
    settings::{RedactedError, SettingsError},
    startup::{StartingSupervisor, Startup, StartupCause, StartupError},
};
use std::{
    io,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

#[derive(Default)]
struct Evidence {
    acquisitions: AtomicUsize,
    spawns: AtomicUsize,
    cleanup: Mutex<Vec<&'static str>>,
}

// Application fixture composes the same validated native constructors with
// real local files. No second configuration validator or resource framework.
fn start(
    pairs: &[(&str, &str)],
    directory: PathBuf,
    evidence: Arc<Evidence>,
) -> Result<
    (
        StartingSupervisor<RedactedError<io::Error>>,
        batter::lifecycle::ShutdownHandle,
    ),
    SettingsError,
> {
    let settings = load(pairs)?;
    let second = Duration::from_secs(1);
    let cleanup = CleanupBudget::new(second, second, second).unwrap();
    let supervisor = settings
        .supervisor(ShutdownBudget::new(second, second, second, cleanup).unwrap())
        .unwrap();
    let handle = supervisor.handle();
    let starting = Startup::new(
        supervisor,
        OperationContext::new(Duration::from_secs(5)).unwrap(),
        cleanup,
        move |scope| {
            Box::pin(async move {
                for name in ["first", "second"] {
                    let slot = scope.supervisor().reserve_cleanup(name).unwrap();
                    evidence.acquisitions.fetch_add(1, Ordering::SeqCst);
                    let path = directory.join(name);
                    let file = std::fs::File::create(&path)
                        .map_err(|e| RedactedError::new("file", "acquisition failed", e))?;
                    let closed = evidence.clone();
                    slot.register(move || async move {
                        drop(file);
                        closed.cleanup.lock().unwrap().push(name);
                        std::fs::remove_file(path)
                            .map_err(|error| Box::new(error) as batter::BoxError)
                    });
                }
                // Simulate external removal: the second finalizer will really fail.
                std::fs::remove_file(directory.join("second")).unwrap();
                scope.stage("third.acquire").unwrap();
                let _slot = scope.supervisor().reserve_cleanup("third").unwrap();
                evidence.acquisitions.fetch_add(1, Ordering::SeqCst);
                let missing = std::fs::File::open(directory.join("missing-secret-marker"));
                // No finalizer or application task may be registered after failed acquisition.
                let _file =
                    missing.map_err(|e| RedactedError::new("file", "acquisition failed", e))?;
                evidence.spawns.fetch_add(1, Ordering::SeqCst);
                scope
                    .supervisor()
                    .register("application", |signal| async move {
                        signal.mark_started();
                        signal.draining().await;
                        Ok(())
                    })
                    .unwrap();
                Ok(())
            })
        },
    )
    .start();
    Ok((starting, handle))
}

#[tokio::test]
async fn partial_startup_retains_real_failure_and_all_lifo_finalizer_outcomes() {
    use std::error::Error;
    let temporary = temp_dir::TempDir::new().unwrap();
    let directory = temporary.path().to_owned();
    let evidence = Arc::new(Evidence::default());
    let (mut starting, handle) = start(&[], directory.clone(), evidence.clone()).unwrap();
    let result = starting.wait().await;
    let remaining: Vec<_> = std::fs::read_dir(&directory).unwrap().collect();
    temporary.close().unwrap();
    let Err(StartupError::Failed(report)) = result else {
        panic!("expected startup report");
    };
    assert_eq!(handle.readiness(), Readiness::Draining); // No running driver was transferred.
    assert_eq!(evidence.acquisitions.load(Ordering::SeqCst), 3);
    assert_eq!(evidence.spawns.load(Ordering::SeqCst), 0);
    assert_eq!(*evidence.cleanup.lock().unwrap(), ["second", "first"]);
    assert!(remaining.is_empty());
    let StartupCause::Failed(cause) = &report.cause else {
        panic!("missing primary failure");
    };
    assert_eq!(cause.cause().kind(), io::ErrorKind::NotFound);
    assert!(cause.source().unwrap().is::<io::Error>());
    assert_eq!(report.stage, "third.acquire");
    assert_eq!(report.cleanup.records.len(), 2); // Failed third acquisition never registered.
    assert_eq!(report.cleanup.records[0].name, "second");
    assert_eq!(report.cleanup.records[0].outcome, CleanupOutcome::Failed);
    assert_eq!(
        report.cleanup.records[0]
            .error
            .as_ref()
            .unwrap()
            .downcast_ref::<io::Error>()
            .unwrap()
            .kind(),
        io::ErrorKind::NotFound
    );
    assert_eq!(report.cleanup.records[1].name, "first");
    assert_eq!(report.cleanup.records[1].outcome, CleanupOutcome::Succeeded);
    assert!(!report.cleanup.is_success());
    let formatted = format!("{report} {report:#?} {cause} {cause:#?}");
    assert!(formatted.contains("third.acquire"));
    assert!(!formatted.contains("secret-marker"));
}

#[tokio::test]
async fn invalid_settings_prevent_acquisition_and_application_spawn() {
    let evidence = Arc::new(Evidence::default());
    let result = start(
        &[("BATTER_POOL_MIN_CONNECTIONS", "9")],
        PathBuf::from("/unused"),
        evidence.clone(),
    );
    assert!(matches!(result, Err(error) if error.field() == "BATTER_POOL_MIN_CONNECTIONS"));
    assert_eq!(evidence.acquisitions.load(Ordering::SeqCst), 0);
    assert_eq!(evidence.spawns.load(Ordering::SeqCst), 0);
    assert!(evidence.cleanup.lock().unwrap().is_empty());
}
