use super::*;
use batter::operation::OperationPhases;
use batter::runledger::native::core::jobs::{
    JobContext, JobExecutionError, JobExecutionServices, JobExecutionUpdate, JobFailureKind,
    JobInvocation, JobInvocationOwner,
};
use tokio::time::{Instant, advance};

const INVOCATION_TIME: Duration = Duration::from_secs(10);

struct Services {
    deadline: std::time::Instant,
    invocation: JobInvocation,
}

#[async_trait]
impl JobExecutionServices for Services {
    fn deadline(&self) -> std::time::Instant {
        self.deadline
    }

    fn remaining_budget(&self) -> Duration {
        self.deadline
            .saturating_duration_since(Instant::now().into_std())
    }

    fn invocation(&self) -> Option<JobInvocation> {
        Some(self.invocation.clone())
    }

    async fn persist_progress(&self, _: JobExecutionUpdate<'_>) -> Result<(), JobExecutionError> {
        panic!("the greeting operations do not persist native progress")
    }
}

fn phases() -> (JobInvocationOwner, OperationPhases) {
    let owner = JobInvocationOwner::new();
    let services = Services {
        deadline: (Instant::now() + INVOCATION_TIME).into_std(),
        invocation: owner.invocation(),
    };
    let context: JobContext = serde_json::from_value(json!({
        "job_id": "00000000-0000-0000-0000-000000000001",
        "run_number": 1,
        "attempt": 1,
        "organization_id": null,
        "worker_id": "consumer-test",
        "checkpoint": null,
    }))
    .unwrap();
    let phases = job_phases(JobExecution::new(&context, &services), FINAL_STATE_RESERVE).unwrap();
    (owner, phases)
}

#[tokio::test(start_paused = true)]
async fn expired_work_is_interruption_while_finalization_time_remains() {
    let (_owner, phases) = phases();
    advance(INVOCATION_TIME - FINAL_STATE_RESERVE).await;
    assert!(phases.finalization().check().is_ok());

    let failure = greeting_name(phases.work(), json!({"name": "valid"}))
        .await
        .unwrap_err();
    assert_eq!(failure.kind, JobFailureKind::Timeout);
    assert_eq!(failure.code, "facade.consumer.work_interrupted");
    assert!(phases.finalization().check().is_ok());
}

#[tokio::test(start_paused = true)]
async fn cancellation_is_distinct_from_payload_and_observer_failures() {
    let (owner, phases) = phases();
    phases.cancel_work();
    let failure = greeting_name(phases.work(), json!({"name": "valid"}))
        .await
        .unwrap_err();
    assert_eq!(failure.kind, JobFailureKind::Timeout);
    assert_eq!(failure.code, "facade.consumer.work_interrupted");
    assert!(phases.finalization().check().is_ok());

    drop(owner);
    let (executed, mut received) = unbounded_channel();
    let failure = record_greeting(phases.finalization(), executed, "valid".into())
        .await
        .unwrap_err();
    assert_eq!(failure.kind, JobFailureKind::Timeout);
    assert_eq!(failure.code, "facade.consumer.record_interrupted");
    assert!(received.recv().await.is_none());
}

#[tokio::test(start_paused = true)]
async fn expired_finalization_does_not_claim_the_observer_failed() {
    let (_owner, phases) = phases();
    advance(INVOCATION_TIME).await;
    let (executed, mut received) = unbounded_channel();
    let failure = record_greeting(phases.finalization(), executed, "valid".into())
        .await
        .unwrap_err();
    assert_eq!(failure.kind, JobFailureKind::Timeout);
    assert_eq!(failure.code, "facade.consumer.record_interrupted");
    assert!(received.recv().await.is_none());
}

#[tokio::test(start_paused = true)]
async fn application_failures_and_success_keep_their_meaning() {
    let (_owner, phases) = phases();
    let missing = greeting_name(phases.work(), json!({})).await.unwrap_err();
    assert_eq!(missing.kind, JobFailureKind::Terminal);
    assert_eq!(missing.code, "facade.consumer.payload");

    let name = greeting_name(phases.work(), json!({"name": "valid"}))
        .await
        .unwrap();
    let (executed, mut received) = unbounded_channel();
    record_greeting(phases.finalization(), executed.clone(), name.clone())
        .await
        .unwrap();
    assert_eq!(received.recv().await.as_deref(), Some("valid"));
    drop(received);
    let closed = record_greeting(phases.finalization(), executed, name)
        .await
        .unwrap_err();
    assert_eq!(closed.kind, JobFailureKind::Terminal);
    assert_eq!(closed.code, "facade.consumer.observer");
}
