//! Real-backend producer, launched in isolated processes by the Docker harness.
#![allow(
    clippy::unwrap_used,
    reason = "a panic fails the bounded integration process"
)]

use batter_core::{
    cleanup::CleanupBudget,
    lifecycle::{ShutdownBudget, Supervisor},
    operation::OperationOwner,
    service::{self, DiagnosticOutcome, ServiceOutcome},
    startup::Startup,
};
use batter_otlp::{
    Schedule,
    diagnostics::{Closure, ExportOutcome, MetricsExport},
    prepare_victoriametrics,
};
use std::{convert::Infallible, time::Duration};

#[tokio::test]
#[ignore = "run python3 scripts/check_victoriametrics.py to provision the pinned backend"]
async fn exports_cumulative_metrics_through_service_completion() {
    tokio::time::timeout(Duration::from_secs(15), export())
        .await
        .expect("service and diagnostic completion are bounded");
}

async fn export() {
    let endpoint = std::env::var("BATTER_TEST_VM_ENDPOINT").unwrap();
    let instance = std::env::var("BATTER_TEST_VM_INSTANCE").unwrap();
    let count: usize = std::env::var("BATTER_TEST_VM_COUNT")
        .unwrap()
        .parse()
        .unwrap();
    assert!(count > 1, "the final snapshot must add observations");
    let second = Duration::from_secs(1);
    let cleanup = CleanupBudget::new(second, second, second).unwrap();
    let supervisor = Supervisor::new(ShutdownBudget::new(second, second, second, cleanup).unwrap());
    let startup = Startup::protected(
        supervisor,
        OperationOwner::new(second).unwrap().into_context(),
        cleanup,
        async |scope| {
            scope
                .reserve_cleanup("resource")?
                .register(|| async { Ok(()) });
            scope.registration().register("worker", |startup| async {
                let running = startup.acknowledge_started();
                running.draining().await;
                Ok(running.stopped())
            })?;
            Ok::<_, batter_core::RegistrationError>(())
        },
    );
    let diagnostics = prepare_victoriametrics(
        &endpoint,
        "example-service",
        &instance,
        Schedule::new(Duration::from_millis(50), second, second * 2).unwrap(),
    )
    .unwrap();
    let owner = service::start(startup, diagnostics);
    owner.wait_ready().await.unwrap();
    observe().await;
    // Allow periodic snapshots of count=1 before adding the final observations.
    // The report must confirm a periodic acknowledgement; elapsed time alone
    // is not accepted as evidence of export.
    tokio::time::sleep(Duration::from_millis(250)).await;
    for _ in 1..count {
        observe().await;
    }
    owner.request_shutdown();
    let completion = owner.wait().await;
    assert!(
        matches!(completion.service(), ServiceOutcome::Shutdown(Ok(_))),
        "service must finish cleanly"
    );
    let DiagnosticOutcome::Completed(MetricsExport::Exported(report)) = completion.diagnostics()
    else {
        panic!("export did not complete: {:?}", completion.diagnostics());
    };
    assert!(
        report.periodic.acknowledged > 0,
        "periodic export must be observed"
    );
    assert_eq!(report.periodic.failed, 0, "no periodic export failed");
    assert_eq!(
        report.final_export,
        ExportOutcome::Acknowledged,
        "final snapshot acknowledged"
    );
    assert_eq!(
        report.coverage,
        service::CompletionCoverage::Reported,
        "complete service report retained"
    );
    assert_eq!(report.closure.exporter, Closure::Closed, "exporter closed");
    assert_eq!(report.closure.provider, Closure::Closed, "provider closed");
    // The outer harness queries VictoriaMetrics after this process exits.
}

async fn observe() {
    OperationOwner::new(Duration::from_secs(1))
        .unwrap()
        .context()
        .run("live.operation", |_| async {
            tokio::time::sleep(Duration::from_millis(2)).await;
            Ok::<_, Infallible>(())
        })
        .await
        .unwrap();
}
