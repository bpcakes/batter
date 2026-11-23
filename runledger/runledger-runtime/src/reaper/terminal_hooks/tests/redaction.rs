use super::super::{
    HookMetadata, TerminalHookFanout, warn_join_cancelled_missing_metadata,
    warn_join_failed_missing_metadata, warn_join_panic_missing_metadata,
};
use super::*;
use crate::dead_letter_hook::DeadLetterHookOutcome;
use crate::test_logs::{
    LogBuffer, PANIC_SENTINEL, SentinelPayload, capture_logs, panicked_join_error,
};

struct PanickingDeadLetterHandler(SentinelPayload);

#[async_trait::async_trait]
impl JobHandler for PanickingDeadLetterHandler {
    fn job_type(&self) -> JobType<'static> {
        JobType::new(HOOK_JOB_TYPE)
    }

    async fn execute(
        &self,
        _context: JobContext,
        _payload: Value,
    ) -> Result<JobCompletion, JobFailure> {
        Ok(JobCompletion::success())
    }

    async fn on_dead_letter(
        &self,
        _context: JobContext,
        _payload: Value,
        _dead_letter: JobDeadLetterInfo,
    ) {
        self.0.raise();
    }
}

fn hook_metadata() -> HookMetadata {
    HookMetadata {
        job_id: Uuid::nil().to_string(),
        job_type: HOOK_JOB_TYPE.to_owned(),
        run_number: RUN_NUMBER,
        attempt: ATTEMPT,
    }
}

fn assert_join_classified(log: &LogBuffer, message: &str) {
    log.assert_redacted_event(message);
    let text = log.text();
    assert!(text.contains("task_id="), "{text}");
    assert!(text.contains("is_panic=true"), "{text}");
    assert!(text.contains("is_cancelled=false"), "{text}");
}

#[tokio::test]
async fn terminal_hook_panic_logs_omit_the_payload() {
    for payload in SentinelPayload::ALL {
        let log = LogBuffer::default();
        let _capture = capture_logs(&log);
        let mut registry = JobRegistry::new();
        registry.register(PanickingDeadLetterHandler(payload));
        let jobs = vec![reaped_terminal_lease(
            Uuid::now_v7(),
            None,
            Some("worker-that-lost-the-lease".to_owned()),
            None,
        )];

        let (_shutdown_tx, mut shutdown_rx) = watch::channel(false);
        let result =
            notify_handlers_of_terminal_lease_expirations(&registry, &jobs, &mut shutdown_rx).await;

        assert_eq!(result, TerminalHookFanoutResult::Completed { started: 1 });
        log.assert_redacted_event("terminal failure hook panicked; continuing reaper loop");
    }
}

#[tokio::test]
async fn terminal_hook_result_logs_omit_the_payload() {
    for payload in SentinelPayload::ALL {
        for (with_metadata, message) in [
            (
                true,
                "terminal failure hook panicked; continuing reaper loop",
            ),
            (
                false,
                "terminal failure hook panicked; metadata missing in reaper loop",
            ),
        ] {
            let (_shutdown_tx, shutdown_rx) = watch::channel(false);
            let mut fanout = TerminalHookFanout::with_registry(
                &shutdown_rx,
                crate::settlement::TaskRegistry::default(),
            );
            let error = panicked_join_error(payload).await;
            let id = error.id();

            if with_metadata {
                fanout.metadata.insert(id, hook_metadata());
            }
            let log = LogBuffer::default();
            let capture = capture_logs(&log);
            fanout.handle_next_hook_result(Ok((
                id,
                DeadLetterHookOutcome::Panicked(PANIC_SENTINEL.to_owned()),
            )));
            log.assert_redacted_event(message);
            drop(capture);

            if with_metadata {
                fanout.metadata.insert(id, hook_metadata());
            }
            let log = LogBuffer::default();
            let _capture = capture_logs(&log);
            fanout.handle_next_hook_result(Err(Arc::new(error)));
            assert_join_classified(&log, message);
        }
    }
}

#[tokio::test]
async fn terminal_hook_join_helpers_omit_the_payload() {
    type Helper = fn(&tokio::task::JoinError);
    let helpers: [(Helper, &str); 6] = [
        (
            |error| hook_metadata().warn_join_panic(error),
            "terminal failure hook panicked; continuing reaper loop",
        ),
        (
            |error| hook_metadata().warn_join_cancelled(error),
            "terminal failure hook was cancelled; continuing reaper loop",
        ),
        (
            |error| hook_metadata().warn_join_failed(error),
            "terminal failure hook join failed; continuing reaper loop",
        ),
        (
            warn_join_panic_missing_metadata,
            "terminal failure hook panicked; metadata missing in reaper loop",
        ),
        (
            warn_join_cancelled_missing_metadata,
            "terminal failure hook was cancelled; metadata missing in reaper loop",
        ),
        (
            warn_join_failed_missing_metadata,
            "terminal failure hook join failed; metadata missing in reaper loop",
        ),
    ];
    for payload in SentinelPayload::ALL {
        // Each helper is safe even when handed a join error it does not expect.
        for (helper, message) in helpers {
            let error = panicked_join_error(payload).await;
            let log = LogBuffer::default();
            let _capture = capture_logs(&log);
            helper(&error);
            assert_join_classified(&log, message);
        }
    }
}
