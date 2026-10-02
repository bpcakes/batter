//! Owner, waiter and observer semantics of the serving orchestration.

use super::ServiceOwner;
use crate::diagnostics::MetricsExport;
#[cfg(feature = "metrics-export")]
use crate::diagnostics::{
    Closure, DiagnosticClosure, ExportFailure, ExportHistory, ExportOutcome, ExportReport,
    FinalCoverage, GuardRejections,
};
use batter::{
    cleanup::CleanupBudget,
    lifecycle::{Readiness, ShutdownBudget, Supervisor},
};
use std::{process::ExitCode, time::Duration};
use tokio::sync::oneshot;

const PRIVATE: &str = "private-marker";

fn supervisor() -> Supervisor {
    let second = Duration::from_secs(1);
    let cleanup = CleanupBudget::new(second * 5, second * 5, second).unwrap();
    Supervisor::new(ShutdownBudget::new(second, second, second, cleanup).unwrap())
}

#[cfg(feature = "metrics-export")]
fn report(final_export: ExportOutcome) -> MetricsExport {
    MetricsExport::Exported(ExportReport {
        periodic: ExportHistory::default(),
        final_export,
        coverage: FinalCoverage::Reported,
        closure: DiagnosticClosure {
            exporter: Closure::Closed,
            provider: Closure::Closed,
        },
        rejected: GuardRejections::default(),
    })
}

/// A started service whose cleanup waits for `release`, and an owner for it.
fn owned(release: oneshot::Receiver<()>) -> (ServiceOwner, batter::lifecycle::LifecycleStatus) {
    let mut supervisor = supervisor();
    supervisor
        .register("component", |startup| async move {
            let shutdown = startup.acknowledge_started();
            shutdown.draining().await;
            Ok(shutdown.stopped())
        })
        .unwrap();
    supervisor
        .on_cleanup("postgres.pool", move || async move {
            let _ = release.await;
            Ok(())
        })
        .unwrap();
    let handle = supervisor.handle();
    let status = handle.status();
    let startup = batter::startup::Startup::scoped(
        supervisor,
        batter::operation::OperationOwner::new(Duration::from_secs(1))
            .unwrap()
            .into_context(),
        super::super::cleanup_budget(),
        |_| Box::pin(async { Ok::<_, super::super::InitializationFailure>(()) }),
    );
    (
        ServiceOwner::new(batter::service::start(
            startup,
            crate::config::PreparedMetrics::Disabled,
        )),
        status,
    )
}

#[test]
fn waiter_cancellation_requests_nothing_and_later_observers_receive_the_completion() {
    for runtime in [
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap(),
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap(),
    ] {
        runtime.block_on(async {
            let (release, released) = oneshot::channel();
            let (owner, status) = owned(released);
            status.wait_ready().await.unwrap();
            let cancelled = tokio::time::timeout(Duration::from_millis(50), owner.wait()).await;
            assert!(cancelled.is_err());
            assert_eq!(status.readiness(), Readiness::Ready);
            let observer = owner.observer();
            // Owner loss requests ordinary drain; cleanup stays owned.
            drop(owner);
            assert!(
                tokio::time::timeout(Duration::from_millis(50), observer.wait())
                    .await
                    .is_err()
            );
            release.send(()).unwrap();
            let completion = observer.wait().await;
            assert!(completion.service().is_ok());
            assert_eq!(completion.exit_code(), ExitCode::SUCCESS);
            let later = observer.clone();
            assert!(later.wait().await.service().is_ok());
            assert_eq!(status.readiness(), Readiness::Stopped);
        });
    }
}

struct Diagnostic {
    report: MetricsExport,
    panic: bool,
}
impl batter::service::Diagnostics for Diagnostic {
    type Report = MetricsExport;
    async fn install(self, completion: batter::service::DiagnosticCompletion) -> MetricsExport {
        let _ = completion.wait().await;
        assert!(!self.panic, "{PRIVATE}");
        self.report
    }
}

#[tokio::test]
async fn diagnostic_panic_cannot_change_the_service_exit_code() {
    for failed in [false, true] {
        let mut process = supervisor();
        process
            .on_cleanup("postgres.pool", || async { Ok(()) })
            .unwrap();
        process
            .register("component", |startup| async move {
                let running = startup.acknowledge_started();
                running.draining().await;
                Ok(running.stopped())
            })
            .unwrap();
        let status = process.status();
        let handle = process.handle();
        let startup = batter::startup::Startup::scoped(
            process,
            batter::operation::OperationOwner::new(Duration::from_secs(1))
                .unwrap()
                .into_context(),
            super::super::cleanup_budget(),
            move |_| {
                Box::pin(async move {
                    if failed {
                        Err(super::super::initialization(std::io::Error::other(PRIVATE)))
                    } else {
                        Ok(())
                    }
                })
            },
        );
        let owner = ServiceOwner::new(batter::service::start(
            startup,
            Diagnostic {
                report: MetricsExport::Disabled,
                panic: true,
            },
        ));
        if !failed {
            status.wait_ready().await.unwrap();
            handle.request();
        }
        let completion = owner.wait().await;
        assert_eq!(completion.metrics(), &MetricsExport::Abandoned);
        assert_eq!(completion.service().is_err(), failed);
        assert_eq!(
            completion.exit_code(),
            if failed {
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            }
        );
        assert!(matches!(
            completion.foundation().diagnostics(),
            batter::service::DiagnosticOutcome::TaskFailed(_)
        ));
        if failed {
            let original = completion
                .service()
                .unwrap_err()
                .error()
                .downcast_ref::<super::super::ProtectedRuntimeStartupFailure>()
                .unwrap();
            assert!(matches!(
                original.startup(),
                batter::startup::StartupError::Failed(_)
            ));
        }
        assert!(!format!("{completion:?} {completion}").contains(PRIVATE));
    }
}

#[cfg(feature = "metrics-export")]
#[tokio::test]
async fn collector_failure_keeps_service_success() {
    let mut process = supervisor();
    process
        .on_cleanup("postgres.pool", || async { Ok(()) })
        .unwrap();
    process
        .register("component", |startup| async move {
            let running = startup.acknowledge_started();
            running.draining().await;
            Ok(running.stopped())
        })
        .unwrap();
    let status = process.status();
    let handle = process.handle();
    let startup = batter::startup::Startup::scoped(
        process,
        batter::operation::OperationOwner::new(Duration::from_secs(1))
            .unwrap()
            .into_context(),
        super::super::cleanup_budget(),
        |_| Box::pin(async { Ok::<_, super::super::InitializationFailure>(()) }),
    );
    let failed = ExportOutcome::Failed(ExportFailure::Status(503));
    let owner = ServiceOwner::new(batter::service::start(
        startup,
        Diagnostic {
            report: report(failed),
            panic: false,
        },
    ));
    status.wait_ready().await.unwrap();
    handle.request();
    let completion = owner.wait().await;
    assert_eq!(completion.exit_code(), ExitCode::SUCCESS);
    let MetricsExport::Exported(retained) = completion.metrics() else {
        panic!("lost export report")
    };
    assert_eq!(retained.final_export, failed);
}
