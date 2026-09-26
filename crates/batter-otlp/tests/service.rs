//! Public adapter composition in a database-free worker root.
#[path = "support/collector.rs"]
mod collector;

use batter_core::{
    cleanup::CleanupBudget,
    lifecycle::{ShutdownBudget, Supervisor},
    operation::OperationOwner,
    service::{self, DiagnosticOutcome, ServiceOutcome},
    startup::Startup,
};
use batter_otlp::{
    Schedule,
    diagnostics::{Closure, ExportFailure, ExportOutcome, MetricsExport},
    prepare,
};
use opentelemetry_proto::tonic::collector::metrics::v1::ExportMetricsServiceRequest;
use std::time::Duration;

#[tokio::test]
async fn owned_worker_exports_after_cleanup_and_second_installation_is_rejected() {
    let collector = collector::Collector::start(collector::Behavior::StallHeaders)
        .await
        .unwrap();
    let second = Duration::from_secs(1);
    let cleanup = CleanupBudget::new(second, second, second).unwrap();
    for first in [true, false] {
        let process =
            Supervisor::new(ShutdownBudget::new(second, second, second, cleanup).unwrap());
        let status = process.status();
        let (cleaned, cleaning) = tokio::sync::oneshot::channel();
        let startup = Startup::scoped(
            process,
            OperationOwner::new(second).unwrap().into_context(),
            cleanup,
            move |scope| {
                Box::pin(async move {
                    scope.reserve_cleanup("resource")?.register(move || async {
                        cleaned.send(()).unwrap();
                        Ok(())
                    });
                    scope
                        .registration()
                        .register("worker", |startup| async move {
                            let running = startup.acknowledge_started();
                            running.draining().await;
                            Ok(running.stopped())
                        })?;
                    Ok::<_, batter_core::RegistrationError>(())
                })
            },
        );
        let prepared = prepare(
            &collector.endpoint(),
            "example-worker",
            Schedule::new(Duration::from_millis(50), second * 2, second).unwrap(),
        )
        .unwrap();
        let owner = service::start(startup, prepared);
        status.wait_ready().await.unwrap();
        if first {
            OperationOwner::new(second)
                .unwrap()
                .context()
                .run("worker.prepare", |_| async {
                    Ok::<_, std::convert::Infallible>(())
                })
                .await
                .unwrap();
            collector.wait_requests(1, second).await.unwrap();
        }
        owner.request_shutdown();
        cleaning.await.unwrap();
        if first {
            // Cleanup is complete, but the periodic request is still in flight.
            // Finalization must retain it under its original attempt deadline.
            assert!(
                tokio::time::timeout(Duration::from_millis(25), owner.wait())
                    .await
                    .is_err()
            );
            assert_eq!(collector.requests().len(), 1, "no early final export");
            collector.set(collector::Behavior::Accept);
            collector.release(); // The stalled periodic request returns 503.
        }
        let completion = tokio::time::timeout(second * 3, owner.wait())
            .await
            .unwrap();
        assert!(matches!(
            completion.service(),
            ServiceOutcome::Shutdown(Ok(_))
        ));
        match completion.diagnostics() {
            DiagnosticOutcome::Completed(MetricsExport::Exported(report)) if first => {
                assert_eq!(report.final_export, ExportOutcome::Acknowledged);
                assert_eq!(report.coverage, service::CompletionCoverage::Reported);
                assert_eq!(report.periodic.attempts, 1);
                assert_eq!(report.periodic.failed, 1);
                assert_eq!(
                    report.periodic.first_failure,
                    Some(ExportFailure::Status(503))
                );
                assert_eq!(report.closure.exporter, Closure::Closed);
                assert_eq!(report.closure.provider, Closure::Closed);
            }
            DiagnosticOutcome::Completed(MetricsExport::InstallationRejected { closure })
                if !first =>
            {
                assert_eq!(closure.exporter, batter_otlp::diagnostics::Closure::Closed);
                assert_eq!(closure.provider, batter_otlp::diagnostics::Closure::Closed);
            }
            other => panic!("unexpected diagnostics: {other:?}"),
        }
    }
    let requests = collector.requests();
    assert_eq!(
        requests.len(),
        2,
        "one periodic request, then one final request"
    );
    assert_final_metrics(&requests[1]);
    collector.close().await.unwrap();
}

fn assert_final_metrics(request: &ExportMetricsServiceRequest) {
    let series = collector::series(request);
    assert_eq!(
        collector::counter(
            &series,
            batter_core::telemetry::metrics::CLEANUP_HOOKS,
            &[("outcome", "succeeded")]
        ),
        Some(1)
    );
    assert_eq!(
        collector::counter(
            &series,
            batter_core::telemetry::metrics::SHUTDOWNS,
            &[("cause", "requested"), ("result", "success")]
        ),
        Some(1)
    );
}
