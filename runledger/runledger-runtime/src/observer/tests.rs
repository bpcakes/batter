use super::*;
use crate::test_logs::{LogBuffer, PANIC_SENTINEL, SentinelPayload, capture_logs};

struct PanickingObserver(SentinelPayload);

#[async_trait]
impl JobLifecycleObserver for PanickingObserver {
    async fn on_job_running(&self, _event: JobRunningEvent) {
        self.0.raise();
    }
}

fn observed_job() -> ObservedJob {
    ObservedJob::new(
        Uuid::nil(),
        JobTypeName::from_static("jobs.test.observer.redaction"),
        None,
        1,
        1,
        3,
        "observer-redaction-worker",
    )
}

#[tokio::test]
async fn lifecycle_observer_panic_logs_omit_the_payload() {
    for payload in SentinelPayload::ALL {
        let log = LogBuffer::default();
        let _capture = capture_logs(&log);
        let (shutdown, _) = crate::shutdown::ShutdownSignal::channel();
        let registry = crate::settlement::TaskRegistry::supervised(shutdown.clone());
        shutdown.request();

        JobLifecycleObservers::from_observer(PanickingObserver(payload))
            .with_settlement(registry.clone())
            .job_running(JobRunningEvent::new(observed_job()))
            .await;

        log.assert_redacted_event("job lifecycle observer panicked");
        let (callbacks, _) = registry.callback_snapshot();
        assert!(
            matches!(callbacks.as_slice(), [crate::RuntimeCallbackFailure::Panicked { callback: "on_job_running", message }]
                if message == PANIC_SENTINEL),
            "{payload:?} payload remains explicit callback evidence"
        );
    }
}
