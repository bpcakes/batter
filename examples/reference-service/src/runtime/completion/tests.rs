//! Owner, waiter and observer semantics of the serving orchestration.

use super::{OrchestrationFailure, ServiceCompletion, ServiceOwner};
use crate::diagnostics::{
    Closure, DiagnosticClosure, ExportFailure, ExportHistory, ExportOutcome, ExportReport,
    FinalCoverage, GuardRejections, MetricsExport,
};
use batter::{
    BoxError,
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
        .on_cleanup("resource", move || async move {
            let _ = release.await;
            Ok(())
        })
        .unwrap();
    let handle = supervisor.handle();
    let status = handle.status();
    let running = supervisor.start();
    let orchestration = async move {
        let result = running.wait_checked().await;
        ServiceCompletion::new(
            result.map(|_| ()).map_err(BoxError::from),
            MetricsExport::Disabled,
        )
    };
    (ServiceOwner::spawn(handle, orchestration), status)
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

#[test]
fn an_orchestration_panic_is_published_as_an_abandoned_completion() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let owner = ServiceOwner::spawn(supervisor().handle(), async {
            panic!("{PRIVATE}");
        });
        let completion = owner.wait().await;
        assert_eq!(completion.metrics(), &MetricsExport::Abandoned);
        let failure = completion.service().unwrap_err();
        assert!(
            failure
                .error()
                .downcast_ref::<OrchestrationFailure>()
                .is_some()
        );
        for text in [
            format!("{completion:?}"),
            format!("{completion}"),
            format!("{failure:?} {failure}"),
        ] {
            assert!(!text.contains(PRIVATE), "{text}");
        }
    });
}

#[test]
fn diagnostics_never_change_the_service_result_or_exit_classification() {
    let failed = ExportOutcome::Failed(ExportFailure::Status(503));
    let succeeded = ServiceCompletion::new(Ok(()), report(failed));
    assert!(succeeded.service().is_ok());
    assert_eq!(succeeded.exit_code(), ExitCode::SUCCESS);
    let MetricsExport::Exported(retained) = succeeded.metrics() else {
        panic!("diagnostics are retained separately")
    };
    assert_eq!(retained.final_export, failed);

    for metrics in [report(ExportOutcome::Acknowledged), report(failed)] {
        let error: BoxError = Box::new(std::io::Error::other(PRIVATE));
        let completion = ServiceCompletion::new(Err(error), metrics);
        assert_eq!(completion.exit_code(), ExitCode::FAILURE);
        let original = completion.service().unwrap_err().error();
        assert_eq!(
            original
                .downcast_ref::<std::io::Error>()
                .unwrap()
                .to_string(),
            PRIVATE
        );
        let text = format!("{completion:?} {completion}");
        assert!(!text.contains(PRIVATE), "{text}");
        assert!(text.contains("failed"));
    }
}
