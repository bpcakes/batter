use batter_core::{
    cleanup::{CleanupBudget, CleanupOutcome},
    lifecycle::{Readiness, ShutdownBudget, Supervisor},
    operation::OperationOwner,
    service::{
        self, CompletionCoverage, DiagnosticCompletion, DiagnosticOutcome, Diagnostics,
        ServiceOutcome,
    },
    startup::{InitializationError, Startup, StartupCause, StartupError},
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::oneshot;

const SECOND: Duration = Duration::from_secs(1);
const PRIVATE: &str = "private-diagnostic-marker";

fn cleanup() -> CleanupBudget {
    CleanupBudget::new(SECOND, SECOND, SECOND).unwrap()
}
fn supervisor() -> Supervisor {
    Supervisor::new(ShutdownBudget::new(SECOND, SECOND, SECOND, cleanup()).unwrap())
}

#[derive(Clone, Copy)]
enum PanicAt {
    Never,
    Install,
    FirstPoll,
    Final,
}
struct Diagnostic {
    phase: PanicAt,
    installed: Arc<AtomicBool>,
    final_started: Option<oneshot::Sender<()>>,
    final_release: Option<oneshot::Receiver<()>>,
}
impl Diagnostics for Diagnostic {
    type Report = CompletionCoverage;
    fn install(
        self,
        completion: DiagnosticCompletion,
    ) -> impl Future<Output = Self::Report> + Send {
        assert!(!matches!(self.phase, PanicAt::Install), "{PRIVATE}");
        self.installed.store(true, Ordering::SeqCst);
        async move {
            assert!(!matches!(self.phase, PanicAt::FirstPoll), "{PRIVATE}");
            let witness = completion.wait().await;
            if let Some(entered) = self.final_started {
                let _ = entered.send(());
            }
            if let Some(release) = self.final_release {
                let _ = release.await;
            }
            assert!(!matches!(self.phase, PanicAt::Final), "{PRIVATE}");
            witness.coverage()
        }
    }
}
fn diagnostic(phase: PanicAt) -> Diagnostic {
    Diagnostic {
        phase,
        installed: Arc::new(AtomicBool::new(false)),
        final_started: None,
        final_release: None,
    }
}

#[test]
fn missing_runtime_rejects_before_installation_or_startup() {
    let diagnostics = diagnostic(PanicAt::Never);
    let installed = diagnostics.installed.clone();
    let started = Arc::new(AtomicBool::new(false));
    let observed = started.clone();
    let startup = Startup::scoped(
        supervisor(),
        OperationOwner::new(SECOND).unwrap().into_context(),
        cleanup(),
        move |_| {
            observed.store(true, Ordering::SeqCst);
            Box::pin(async { Ok::<_, std::convert::Infallible>(()) })
        },
    );
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        drop(service::start(startup, diagnostics));
    }));
    assert!(result.is_err());
    assert!(!installed.load(Ordering::SeqCst));
    assert!(!started.load(Ordering::SeqCst));
}

#[tokio::test]
async fn owner_loss_and_cancelled_waiters_preserve_ordered_completion() {
    let process = supervisor();
    let status = process.status();
    let (cleaned, cleaning) = oneshot::channel();
    let (release, released) = oneshot::channel();
    let (final_entered, mut final_wait) = oneshot::channel();
    let (final_release, final_released) = oneshot::channel();
    let mut diagnostics = diagnostic(PanicAt::Never);
    diagnostics.final_started = Some(final_entered);
    diagnostics.final_release = Some(final_released);
    let installed = diagnostics.installed.clone();
    let startup = Startup::scoped(
        process,
        OperationOwner::new(SECOND).unwrap().into_context(),
        cleanup(),
        move |scope| {
            Box::pin(async move {
                assert!(
                    installed.load(Ordering::SeqCst),
                    "installation precedes application work"
                );
                scope
                    .reserve_cleanup("resource")?
                    .register(move || async move {
                        cleaned.send(()).unwrap();
                        released.await.unwrap();
                        Ok(())
                    });
                scope
                    .registration()
                    .register("component", |startup| async move {
                        let running = startup.acknowledge_started();
                        running.draining().await;
                        Ok(running.stopped())
                    })?;
                Ok::<_, batter_core::RegistrationError>(())
            })
        },
    );
    let owner = service::start(startup, diagnostics);
    status.wait_ready().await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(20), owner.wait())
            .await
            .is_err()
    );
    assert_eq!(status.readiness(), Readiness::Ready);
    let observer = owner.observer();
    drop(owner);
    cleaning.await.unwrap();
    assert!(matches!(
        final_wait.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    ));
    assert!(
        tokio::time::timeout(Duration::from_millis(20), observer.wait())
            .await
            .is_err()
    );
    release.send(()).unwrap();
    final_wait.await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(20), observer.wait())
            .await
            .is_err()
    );
    final_release.send(()).unwrap();
    let completion = observer.wait().await;
    let ServiceOutcome::Shutdown(Ok(success)) = completion.service() else {
        panic!("{completion:?}")
    };
    assert_eq!(
        success.report().cleanup.records[0].outcome,
        CleanupOutcome::Succeeded
    );
    assert!(matches!(
        completion.diagnostics(),
        DiagnosticOutcome::Completed(CompletionCoverage::Reported)
    ));
    let later = observer.wait().await;
    assert!(std::ptr::eq(completion.service(), later.service()));
}

#[tokio::test(flavor = "multi_thread")]
async fn diagnostic_panics_preserve_success_and_original_startup_and_shutdown_errors() {
    for phase in [PanicAt::Install, PanicAt::FirstPoll, PanicAt::Final] {
        for failure in ["none", "startup", "shutdown"] {
            let process = supervisor();
            let status = process.status();
            let handle = process.handle();
            let cleaned = Arc::new(AtomicUsize::new(0));
            let counter = cleaned.clone();
            let startup = Startup::scoped(
                process,
                OperationOwner::new(SECOND).unwrap().into_context(),
                cleanup(),
                move |scope| {
                    Box::pin(async move {
                        scope
                            .reserve_cleanup("resource")
                            .unwrap()
                            .register(move || async move {
                                counter.fetch_add(1, Ordering::SeqCst);
                                if failure == "shutdown" {
                                    Err(std::io::Error::other("original-cleanup").into())
                                } else {
                                    Ok(())
                                }
                            });
                        if failure == "startup" {
                            return Err(std::io::Error::other("original-startup"));
                        }
                        scope
                            .registration()
                            .register("component", |startup| async move {
                                let running = startup.acknowledge_started();
                                running.draining().await;
                                Ok(running.stopped())
                            })
                            .unwrap();
                        Ok(())
                    })
                },
            );
            let owner = service::start(startup, diagnostic(phase));
            if failure != "startup" {
                status.wait_ready().await.unwrap();
                handle.request();
            }
            let completion = owner.wait().await;
            assert_eq!(cleaned.load(Ordering::SeqCst), 1);
            match (failure, completion.service()) {
                ("none", ServiceOutcome::Shutdown(Ok(_))) => {}
                ("startup", ServiceOutcome::StartupFailed(StartupError::Failed(report))) => {
                    let StartupCause::Failed(InitializationError::Application(error)) =
                        &report.cause
                    else {
                        panic!("wrong startup failure")
                    };
                    assert_eq!(error.to_string(), "original-startup");
                    assert_eq!(report.cleanup.records[0].outcome, CleanupOutcome::Succeeded);
                }
                (
                    "shutdown",
                    ServiceOutcome::Shutdown(Err(batter_core::lifecycle::ShutdownFailure::Report(
                        report,
                    ))),
                ) => {
                    assert_eq!(
                        report.cleanup.records[0]
                            .error
                            .as_ref()
                            .unwrap()
                            .to_string(),
                        "original-cleanup"
                    );
                }
                _ => panic!("lost original outcome: {completion:?}"),
            }
            match completion.diagnostics() {
                DiagnosticOutcome::InstallationPanicked(payload) => {
                    assert!(payload.try_inspect(|p| p.is::<String>()).unwrap())
                }
                DiagnosticOutcome::TaskFailed(error) => assert!(error.is_panic()),
                _ => panic!("diagnostic panic was lost"),
            }
            assert!(!format!("{completion:?}").contains(PRIVATE));
            assert!(std::ptr::eq(
                completion.service(),
                owner.wait().await.service()
            ));
        }
    }
}

#[tokio::test]
async fn owner_dropped_before_first_poll_still_retains_cleanup_and_diagnostics() {
    let mut process = supervisor();
    let cleaned = Arc::new(AtomicBool::new(false));
    let flag = cleaned.clone();
    process
        .on_cleanup("resource", move || async move {
            flag.store(true, Ordering::SeqCst);
            Ok(())
        })
        .unwrap();
    let startup = Startup::scoped(
        process,
        OperationOwner::new(SECOND).unwrap().into_context(),
        cleanup(),
        |_| {
            Box::pin(async {
                panic!("initializer must not start after owner loss");
                #[allow(unreachable_code)]
                Ok::<_, std::io::Error>(())
            })
        },
    );
    let owner = service::start(startup, diagnostic(PanicAt::Never));
    let observer = owner.observer();
    drop(owner);
    let completion = observer.wait().await;
    assert!(cleaned.load(Ordering::SeqCst));
    let ServiceOutcome::StartupFailed(StartupError::Failed(report)) = completion.service() else {
        panic!("wrong outcome")
    };
    assert!(matches!(report.cause, StartupCause::Draining));
    assert!(matches!(
        completion.diagnostics(),
        DiagnosticOutcome::Completed(CompletionCoverage::Reported)
    ));
}
