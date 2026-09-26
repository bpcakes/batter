//! Child scenarios. Assertion failures panic and fail the parent validation.

use crate::{
    COMPLETE, PRIVATE,
    collector::{Behavior, Collector, Value, find, series},
};
use batter::{
    cleanup::CleanupOutcome,
    settings::SettingsSource,
    startup::{InitializationError, StartupCause, StartupError},
    telemetry::metrics::{self as catalog, facade::NoopRecorder},
};
use batter_example_reference_service::{
    config::ServingSettings,
    diagnostics::{
        Closure, DiagnosticClosure, ExportFailure, ExportOutcome, ExportReport, FinalCoverage,
        GuardRejections, MetricsExport,
    },
    runtime::{self, InitializationFailure, ProtectedRuntimeStartupFailure, ServiceCompletion},
};
use std::{future::Future, process::ExitCode, time::Duration};
use tokio::{net::TcpListener, time::timeout};

const CLOSED: DiagnosticClosure = DiagnosticClosure {
    exporter: Closure::Closed,
    provider: Closure::Closed,
};

pub fn dispatch(scenario: &str) {
    match scenario {
        "startup-export-current" => current(startup_export()),
        "startup-export-multi" => multi(startup_export()),
        "rejected-installation" => current(rejected_installation()),
        "never-polled" => current(never_polled()),
        "owner-loss-current" => current(owner_loss()),
        "owner-loss-multi" => multi(owner_loss()),
        _ => panic!("unknown metrics fixture"),
    }
    eprintln!("{COMPLETE}");
}

fn current(future: impl Future<Output = ()>) {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(future);
}

fn multi(future: impl Future<Output = ()>) {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap()
        .block_on(future);
}

/// Injected serving settings; the child environment is empty.
pub fn settings(database: &str, endpoint: &str, acquire_ms: &str) -> ServingSettings {
    let pairs = [
        (
            "DATABASE_URL",
            format!("postgres://user:{PRIVATE}@{database}/database?sslmode=disable"),
        ),
        ("JOBS_WORKER_ID", "metrics-worker".to_owned()),
        (
            "BATTER_AUTH_OWNER_ID",
            "00000000-0000-0000-0000-000000000001".to_owned(),
        ),
        ("BATTER_AUTH_TOKEN", "fake-metrics-token".to_owned()),
        ("BATTER_PROVIDER_BASE_URL", "http://127.0.0.1:9/".to_owned()),
        ("BATTER_PROVIDER_TOKEN", "fake-provider-token".to_owned()),
        ("BATTER_BIND", "127.0.0.1:0".to_owned()),
        ("BATTER_POOL_ACQUIRE_TIMEOUT_MS", acquire_ms.to_owned()),
        ("BATTER_METRICS_OTLP_ENDPOINT", endpoint.to_owned()),
    ];
    let overrides =
        SettingsSource::from_pairs(pairs.map(|(key, value)| (key.into(), value.into()))).unwrap();
    ServingSettings::from_sources(None, SettingsSource::default(), overrides).unwrap()
}

pub async fn refused_address() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    address.to_string()
}

/// The protected startup failure retained unchanged beside diagnostics.
type Cause = StartupCause<InitializationError<InitializationFailure>>;

fn startup_failure(completion: &ServiceCompletion, cause: fn(&Cause) -> bool) {
    assert_eq!(completion.exit_code(), ExitCode::FAILURE);
    let failure = completion
        .service()
        .expect_err("startup fails without PostgreSQL")
        .error()
        .downcast_ref::<ProtectedRuntimeStartupFailure>()
        .expect("protected startup failure");
    let StartupError::Failed(report) = failure.startup() else {
        panic!("startup coordinator failed")
    };
    assert_eq!(report.stage, "postgres.acquire");
    assert!(cause(&report.cause));
    assert!(report.cleanup.skipped.is_empty());
    assert_eq!(report.cleanup.records.len(), 1);
    assert_eq!(report.cleanup.records[0].name, "postgres.pool");
    assert_eq!(report.cleanup.records[0].outcome, CleanupOutcome::Succeeded);
    assert!(!format!("{completion:?} {completion}").contains(PRIVATE));
}

fn exported(completion: &ServiceCompletion) -> &ExportReport {
    let MetricsExport::Exported(report) = completion.metrics() else {
        panic!("metrics export ran")
    };
    assert_eq!(report.coverage, FinalCoverage::Reported);
    assert_eq!(report.closure, CLOSED);
    assert_eq!(report.rejected, GuardRejections::default());
    assert_eq!(report.periodic.attempts, 0);
    report
}

/// The final request carries the startup cleanup outcome with its canonical
/// catalog description, and no shutdown because no driver ran.
fn assert_cleanup_exported(collector: &Collector) {
    let requests = collector.requests();
    assert_eq!(requests.len(), 1, "exactly one final export");
    let exported = series(&requests[0]);
    let hook = find(
        &exported,
        catalog::CLEANUP_HOOKS,
        &[("outcome", "succeeded")],
    )
    .expect("startup cleanup exported");
    assert_eq!(
        hook.value,
        Value::Sum {
            value: 1,
            monotonic: true
        }
    );
    assert_eq!(hook.description, "Batter cleanup hook outcomes");
    assert_eq!(hook.unit, "1");
    assert!(find(&exported, catalog::SHUTDOWNS, &[]).is_none());
}

async fn startup_export() {
    let collector = Collector::start(Behavior::Accept).await.unwrap();
    let settings = settings(&refused_address().await, &collector.endpoint(), "200");
    let owner = runtime::start(runtime::prepare(settings).unwrap());
    let completion = owner.wait().await;
    startup_failure(&completion, |cause| {
        matches!(cause, StartupCause::Failed(_))
    });
    assert_eq!(
        exported(&completion).final_export,
        ExportOutcome::Acknowledged
    );
    assert_cleanup_exported(&collector);
    collector.close().await.unwrap();
}

async fn rejected_installation() {
    catalog::install(NoopRecorder).expect("first installation");
    let collector = Collector::start(Behavior::Accept).await.unwrap();
    let settings = settings(&refused_address().await, &collector.endpoint(), "200");
    let completion = runtime::run(runtime::prepare(settings).unwrap()).await;
    startup_failure(&completion, |cause| {
        matches!(cause, StartupCause::Failed(_))
    });
    assert_eq!(
        completion.metrics(),
        &MetricsExport::InstallationRejected { closure: CLOSED }
    );
    assert!(collector.requests().is_empty());
    collector.close().await.unwrap();
}

async fn never_polled() {
    let database = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let collector = Collector::start(Behavior::Accept).await.unwrap();
    let address = database.local_addr().unwrap().to_string();
    let prepared = runtime::prepare(settings(&address, &collector.endpoint(), "3000")).unwrap();
    drop(runtime::run(prepared));
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(
        timeout(Duration::from_millis(100), database.accept())
            .await
            .is_err()
    );
    assert!(collector.requests().is_empty());
    catalog::install(NoopRecorder).expect("never-polled serving installed no recorder");
    collector.close().await.unwrap();
}

async fn owner_loss() {
    let database = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let collector = Collector::start(Behavior::StallHeaders).await.unwrap();
    let address = database.local_addr().unwrap().to_string();
    let owner = runtime::start(
        runtime::prepare(settings(&address, &collector.endpoint(), "3000")).unwrap(),
    );
    let observer = owner.observer();
    // Partial setup: the pool cleanup is registered and native acquisition is
    // pending behind a withheld handshake.
    let (socket, _) = timeout(Duration::from_secs(3), database.accept())
        .await
        .unwrap()
        .unwrap();
    assert!(
        timeout(Duration::from_millis(50), owner.wait())
            .await
            .is_err()
    );
    drop(owner);
    // Startup cleanup completed and the final export is in flight.
    collector
        .wait_requests(1, Duration::from_secs(3))
        .await
        .unwrap();
    assert!(
        timeout(Duration::from_millis(50), observer.wait())
            .await
            .is_err()
    );
    collector.release();
    let completion = observer.wait().await;
    startup_failure(&completion, |cause| matches!(cause, StartupCause::Draining));
    assert_eq!(
        exported(&completion).final_export,
        ExportOutcome::Failed(ExportFailure::Status(503))
    );
    assert_cleanup_exported(&collector);
    let later = observer.clone().wait().await;
    assert_eq!(later.metrics(), completion.metrics());
    drop(socket);
    collector.close().await.unwrap();
}
