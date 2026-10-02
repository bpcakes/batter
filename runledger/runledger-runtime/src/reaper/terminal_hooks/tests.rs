use std::sync::{Arc, Mutex};

use runledger_core::jobs::{
    JobCompletion, JobContext, JobDeadLetterInfo, JobDeadLetterOrigin, JobDeadLetterReason,
    JobFailure, JobType, JobTypeName,
};
use runledger_postgres::jobs::test_support::reaped_lease_record_with_checkpoint;
use runledger_postgres::jobs::{ReapedLeaseDisposition, ReapedLeaseRecord};
use serde_json::{Value, json};
use sqlx::types::Uuid;
use tokio::sync::watch;

use super::{TerminalHookFanoutResult, notify_handlers_of_terminal_lease_expirations};
use crate::registry::{JobHandler, JobRegistry};

const HOOK_JOB_TYPE: &str = "jobs.test.reaper.hook.identity";
const RUN_NUMBER: i32 = 2;
const ATTEMPT: i32 = 3;
const MAX_ATTEMPTS: i32 = 3;

type Deliveries = Arc<Mutex<Vec<(JobContext, JobDeadLetterInfo)>>>;

#[derive(Default)]
struct RecordingDeadLetterHandler {
    deliveries: Deliveries,
}

#[async_trait::async_trait]
impl JobHandler for RecordingDeadLetterHandler {
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
        context: JobContext,
        _payload: Value,
        dead_letter: JobDeadLetterInfo,
    ) {
        self.deliveries
            .lock()
            .expect("delivery list lock should not be poisoned")
            .push((context, dead_letter));
    }
}

fn recording_registry() -> (JobRegistry, Deliveries) {
    let handler = RecordingDeadLetterHandler::default();
    let deliveries = handler.deliveries.clone();
    let mut registry = JobRegistry::new();
    registry.register(handler);
    (registry, deliveries)
}

fn reaped_terminal_lease(
    job_id: Uuid,
    organization_id: Option<Uuid>,
    worker_id: Option<String>,
    checkpoint: Option<Value>,
) -> ReapedLeaseRecord {
    reaped_lease_record_with_checkpoint(
        job_id,
        JobTypeName::from_static(HOOK_JOB_TYPE),
        organization_id,
        RUN_NUMBER,
        ATTEMPT,
        MAX_ATTEMPTS,
        checkpoint,
        worker_id,
        false,
        ReapedLeaseDisposition::DeadLetteredTerminal {
            payload: json!({ "kind": "reaped-terminal-identity" }),
        },
    )
}

#[tokio::test]
async fn terminal_hook_receives_the_durable_attempts_worker_identity() {
    let (registry, deliveries) = recording_registry();
    let job_id = Uuid::now_v7();
    let organization_id = Some(Uuid::now_v7());
    let checkpoint = json!({ "cursor": 41 });
    let jobs = vec![reaped_terminal_lease(
        job_id,
        organization_id,
        Some("worker-that-lost-the-lease".to_owned()),
        Some(checkpoint.clone()),
    )];

    let (_shutdown_tx, mut shutdown_rx) = watch::channel(false);
    let result =
        notify_handlers_of_terminal_lease_expirations(&registry, &jobs, &mut shutdown_rx).await;

    assert_eq!(result, TerminalHookFanoutResult::Completed { started: 1 });
    let deliveries = deliveries
        .lock()
        .expect("delivery list lock should not be poisoned");
    assert_eq!(deliveries.len(), 1);
    let (context, dead_letter) = &deliveries[0];

    let recorded_owner = jobs[0]
        .worker_id
        .as_deref()
        .expect("fixture records the durable lease owner");
    assert_eq!(
        context.worker_id, recorded_owner,
        "hook context must carry the worker recorded on the reaped attempt, not a reaper sentinel"
    );
    assert_eq!(context.worker_id, "worker-that-lost-the-lease");
    assert_eq!(context.job_id, job_id);
    assert_eq!(context.run_number, RUN_NUMBER);
    assert_eq!(context.attempt, ATTEMPT);
    assert_eq!(context.organization_id, organization_id);
    assert_eq!(context.checkpoint, Some(checkpoint));

    assert_eq!(
        dead_letter.origin,
        JobDeadLetterOrigin::Reaper,
        "hooks distinguish reaper deliveries by the typed origin"
    );
    assert_eq!(dead_letter.reason, JobDeadLetterReason::LeaseExpired);
    assert_eq!(dead_letter.max_attempts, Some(MAX_ATTEMPTS));
    assert_eq!(dead_letter.failure, jobs[0].failure);
}

#[tokio::test]
async fn terminal_hook_is_skipped_when_the_reaped_lease_records_no_owner() {
    let (registry, deliveries) = recording_registry();
    let jobs = vec![
        reaped_terminal_lease(Uuid::now_v7(), None, None, None),
        reaped_terminal_lease(
            Uuid::now_v7(),
            None,
            Some("worker-with-recorded-owner".to_owned()),
            None,
        ),
    ];

    let (_shutdown_tx, mut shutdown_rx) = watch::channel(false);
    let result =
        notify_handlers_of_terminal_lease_expirations(&registry, &jobs, &mut shutdown_rx).await;

    assert_eq!(
        result,
        TerminalHookFanoutResult::Completed { started: 1 },
        "an ownerless reaped lease must not invent a worker identity or start a hook"
    );
    let deliveries = deliveries
        .lock()
        .expect("delivery list lock should not be poisoned");
    assert_eq!(deliveries.len(), 1);
    assert_eq!(deliveries[0].0.job_id, jobs[1].job_id);
    assert_eq!(deliveries[0].0.worker_id, "worker-with-recorded-owner");
    assert_eq!(deliveries[0].1.origin, JobDeadLetterOrigin::Reaper);
}
