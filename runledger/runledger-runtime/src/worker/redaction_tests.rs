use std::sync::Arc;

use chrono::Utc;
use runledger_core::jobs::{
    JobCompletion, JobContext, JobDeadLetterInfo, JobDeadLetterReason, JobFailure, JobStage,
    JobStatus, JobType, JobTypeName,
};
use serde_json::{Value, json};
use uuid::Uuid;

use super::dead_letter::{notify_handler_of_dead_letter, worker_dead_letter_info};
use super::{log_drained_job_task_result, log_finished_job_task_result};
use crate::dead_letter_hook::DeadLetterHookOutcome;
use crate::registry::{JobHandler, JobRegistry};
use crate::test_logs::{
    LogBuffer, PANIC_SENTINEL, SentinelPayload, capture_logs, panicked_join_error,
};

const JOB_TYPE: &str = "jobs.test.worker.redaction";

struct PanickingDeadLetterHandler(SentinelPayload);

#[async_trait::async_trait]
impl JobHandler for PanickingDeadLetterHandler {
    fn job_type(&self) -> JobType<'static> {
        JobType::new(JOB_TYPE)
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

fn leased_job() -> runledger_postgres::jobs::JobQueueRecord {
    let now = Utc::now();
    runledger_postgres::jobs::JobQueueRecord {
        id: Uuid::now_v7(),
        job_type: JobTypeName::from_static(JOB_TYPE),
        organization_id: None,
        payload: json!({}),
        status: JobStatus::Leased,
        priority: 100,
        run_number: 1,
        attempt: 3,
        max_attempts: 3,
        timeout_seconds: 30,
        next_run_at: now,
        lease_expires_at: Some(now),
        last_heartbeat_at: None,
        worker_id: Some("worker-redaction".to_owned()),
        started_at: Some(now),
        finished_at: None,
        stage: JobStage::Running,
        progress_done: None,
        progress_total: None,
        progress_pct: None,
        checkpoint: None,
        output: None,
        idempotency_key: None,
        status_reason: None,
        last_error_code: None,
        last_error_message: None,
        created_at: now,
        updated_at: now,
    }
}

#[tokio::test]
async fn dead_letter_hook_panic_logs_omit_the_payload() {
    for payload in SentinelPayload::ALL {
        let log = LogBuffer::default();
        let _capture = capture_logs(&log);
        let mut registry = JobRegistry::new();
        registry.register(PanickingDeadLetterHandler(payload));
        let job = leased_job();
        let context = JobContext {
            job_id: job.id,
            run_number: job.run_number,
            attempt: job.attempt,
            organization_id: None,
            worker_id: "worker-redaction".to_owned(),
            checkpoint: None,
        };
        let failure = JobFailure::terminal("job.test.redaction", "controlled failure");
        let dead_letter =
            worker_dead_letter_info(&failure, JobDeadLetterReason::FailureKindNonRetryable, 3);

        let outcome =
            notify_handler_of_dead_letter(&registry, &context, &job, dead_letter, None).await;

        assert_eq!(
            outcome,
            DeadLetterHookOutcome::Panicked(PANIC_SENTINEL.to_owned()),
            "{payload:?} payload remains explicit hook evidence"
        );
        log.assert_redacted_event("dead-letter hook panicked; continuing worker job task");
    }
}

#[tokio::test]
async fn job_task_join_logs_omit_the_payload() {
    for payload in SentinelPayload::ALL {
        for (log_result, message) in [
            (log_finished_job_task_result as fn(_), "job task crashed"),
            (
                log_drained_job_task_result,
                "job task crashed while draining in-flight jobs",
            ),
        ] {
            let error = Arc::new(panicked_join_error(payload).await);
            let task_id = error.id();
            let log = LogBuffer::default();
            let _capture = capture_logs(&log);

            log_result(Err(error));

            log.assert_redacted_event(message);
            let text = log.text();
            assert!(text.contains(&format!("task_id={task_id}")), "{text}");
            assert!(text.contains("is_panic=true"), "{text}");
            assert!(text.contains("is_cancelled=false"), "{text}");
        }
    }
}
