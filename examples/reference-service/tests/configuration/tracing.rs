#[path = "../../../../test-support/dispatch.rs"]
mod dispatch;
use std::{
    io::{self, Write},
    sync::{Arc, Mutex},
};

#[derive(Clone)]
struct Writer(Arc<Mutex<Vec<u8>>>);
impl Write for Writer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn tracing_projects_static_configuration_metadata_without_native_url_warnings() {
    let bytes = Arc::new(Mutex::new(Vec::new()));
    let captured = bytes.clone();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_writer(move || Writer(captured.clone()))
        .finish();
    let dispatcher = dispatch::new(subscriber);
    tracing::dispatcher::with_default(&dispatcher, || {
        let root = crate::load(&[
            (
                "DATABASE_URL",
                "postgres://user:secret-marker%2540@host/db?sslmode=disable",
            ),
            ("JOBS_WORKER_ID", "secret-marker"),
        ])
        .unwrap();
        tracing::info!(settings = ?root, worker = ?root.worker(), "configuration accepted");
        for url in [
            "postgres://user:secret-marker@host/db?sslmode=disable&secret-marker=secret-marker",
            "postgres://user:secret-marker@host/db?sslmode=secret-marker",
        ] {
            let error = crate::load(&[("DATABASE_URL", url)]).unwrap_err();
            tracing::error!(error = %error, alternate = ?error, "configuration rejected");
        }
    });
    let text = String::from_utf8(bytes.lock().unwrap().clone()).unwrap();
    assert!(text.contains("configuration accepted"));
    assert!(text.contains("configuration rejected"));
    assert!(text.contains("DATABASE_URL: unsupported query setting"));
    assert!(text.contains("DATABASE_URL: invalid TLS mode"));
    assert!(!text.contains("secret-marker"));
    assert!(!text.contains("%2540"));
    assert!(!text.contains("ignoring unrecognized connect parameter"));
}

#[test]
fn startup_report_and_cleanup_tracing_hide_retained_secret_bearing_causes() {
    use batter::{
        cleanup::{CleanupBudget, CleanupOutcome},
        lifecycle::{ShutdownBudget, Supervisor},
        operation::OperationContext,
        startup::{Startup, StartupCause, StartupError},
    };
    use std::{error::Error, time::Duration};
    let bytes = Arc::new(Mutex::new(Vec::new()));
    let captured = bytes.clone();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_writer(move || Writer(captured.clone()))
        .finish();
    let dispatcher = dispatch::new(subscriber);
    let report = tracing::dispatcher::with_default(&dispatcher, || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                // Feed an actual native parser error into the startup report boundary.
                // This is a diagnostic fixture, not a root acquiring on invalid config.
                let error = crate::load(&[(
                    "DATABASE_URL",
                    "postgres://user:secret-marker%2540@host/db?sslmode=secret-marker",
                )])
                .unwrap_err();
                let second = Duration::from_secs(1);
                let cleanup = CleanupBudget::new(second, second, second).unwrap();
                let supervisor =
                    Supervisor::new(ShutdownBudget::new(second, second, second, cleanup).unwrap());
                let mut starting = Startup::new(
                    supervisor,
                    OperationContext::new(second).unwrap(),
                    cleanup,
                    move |scope| {
                        Box::pin(async move {
                            scope.stage("diagnostic.fixture").unwrap();
                            scope
                                .supervisor()
                                .on_cleanup("diagnostic.cleanup", || async {
                                    Err(io::Error::other("cleanup-secret-marker").into())
                                })
                                .unwrap();
                            Err::<(), _>(error)
                        })
                    },
                )
                .start();
                let Err(StartupError::Failed(report)) = starting.wait().await else {
                    panic!("expected retained startup failure");
                };
                tracing::error!(report = %report, alternate = ?report, "startup rejected");
                report
            })
    });
    let StartupCause::Failed(error) = &report.cause else {
        panic!("missing typed cause");
    };
    assert!(error.source().unwrap().is::<sqlx::Error>());
    assert!(
        error
            .source()
            .unwrap()
            .to_string()
            .contains("secret-marker")
    );
    assert_eq!(report.cleanup.records.len(), 1);
    assert_eq!(report.cleanup.records[0].outcome, CleanupOutcome::Failed);
    assert_eq!(
        report.cleanup.records[0]
            .error
            .as_ref()
            .unwrap()
            .downcast_ref::<io::Error>()
            .unwrap()
            .to_string(),
        "cleanup-secret-marker"
    );
    let text = String::from_utf8(bytes.lock().unwrap().clone()).unwrap();
    assert!(text.contains("startup rejected"));
    assert!(text.contains("diagnostic.cleanup"));
    assert!(text.contains("failed"));
    for text in [text, format!("{report} {report:#?} {:?}", vec![&report])] {
        assert!(!text.contains("secret-marker"));
        assert!(!text.contains("%2540"));
    }
}
