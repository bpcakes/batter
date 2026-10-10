//! The `records.notify` worker: a bounded provider step, then a final-state
//! write that still runs when the provider step times out.

use crate::records::{NOTIFY_JOB, NotifiedLedger, NotifyPayload};
use batter::operation::{Interruption, OperationError};
use batter::runledger::native::core::jobs::{
    JobCompletion, JobExecution, JobExecutionHandler, JobFailure, JobType,
};
use batter::runledger::native::core::prelude::async_trait;
use batter::runledger::native::runtime::catalog::{JobCatalog, JobCatalogDefinitionOverrides};
use batter::runledger::{JobPhasesRejection, job_phases};
use serde_json::Value;
use std::{convert::Infallible, time::Duration};

/// Time kept inside the worker's deadline for the `notified_at` write.
pub const FINAL_STATE_RESERVE: Duration = Duration::from_millis(500);

/// Executes `records.notify`: simulate a provider call under its own limit,
/// then record `notified_at` under the finalization phase.
pub struct NotifyRecord<L> {
    ledger: L,
    provider_delay: Duration,
    provider_limit: Duration,
}

impl<L> NotifyRecord<L> {
    /// `provider_delay` is how long the simulated provider takes;
    /// `provider_limit` bounds that step inside the work phase.
    pub fn new(ledger: L, provider_delay: Duration, provider_limit: Duration) -> Self {
        Self {
            ledger,
            provider_delay,
            provider_limit,
        }
    }
}

#[async_trait]
impl<L: NotifiedLedger + 'static> JobExecutionHandler for NotifyRecord<L> {
    fn job_type(&self) -> JobType<'static> {
        NOTIFY_JOB
    }

    async fn execute(
        &self,
        execution: JobExecution<'_>,
        payload: Value,
    ) -> Result<JobCompletion, JobFailure> {
        let payload: NotifyPayload = serde_json::from_value(payload).map_err(|_| {
            JobFailure::terminal("records.invalid_payload", "Expected a record_id.")
        })?;
        let phases =
            job_phases(execution, FINAL_STATE_RESERVE).map_err(|rejection| match rejection {
                JobPhasesRejection::Exhausted | JobPhasesRejection::Ended => {
                    JobFailure::timeout("records.no_work_time", "No work time remained.")
                }
                JobPhasesRejection::Unsupported | JobPhasesRejection::Reserve(_) => {
                    JobFailure::terminal("records.unsupported", "Invocation phases unavailable.")
                }
            })?;

        // One bounded step inside work: ends at the earlier of the provider
        // limit and the work deadline, and the invocation's exit cancels it.
        let provider = match phases.work().child(self.provider_limit) {
            Ok(step) => {
                let delay = self.provider_delay;
                step.context()
                    .run("records.provider", |_| async move {
                        tokio::time::sleep(delay).await;
                        Ok::<_, Infallible>(())
                    })
                    .await
            }
            Err(_) => Err(OperationError::Interrupted(Interruption::DeadlineExceeded)),
        };
        // Classify before recording: an interrupted provider call leaves the
        // notification unknown, so the attempt is reported as a timeout.
        let outcome = match provider {
            Ok(()) => Ok(JobCompletion::success()),
            Err(OperationError::Interrupted(_)) => Err(JobFailure::timeout(
                "records.notify_unknown",
                "The provider step did not complete in time.",
            )),
            Err(OperationError::Failed(never)) => match never {},
        };

        // The final-state write runs either way, under the reserved phase.
        self.ledger
            .mark_notified(phases.finalization(), payload.record_id)
            .await
            .map_err(|_| {
                JobFailure::retryable("records.unrecorded", "notified_at not recorded.")
            })?;
        outcome
    }
}

/// The worker's catalog: one handler whose definition is synced at startup so
/// recorded intents are promoted into the queue.
pub fn catalog<L: NotifiedLedger + 'static>(ledger: L) -> JobCatalog {
    JobCatalog::new().handler_with_definition_overrides(
        NotifyRecord::new(ledger, Duration::from_millis(100), Duration::from_secs(5))
            .into_job_handler(),
        JobCatalogDefinitionOverrides::new()
            .max_attempts(5)
            .timeout_seconds(30),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use batter::operation::OperationContext;
    use batter::runledger::native::core::jobs::{
        JobContext, JobExecutionError, JobExecutionServices, JobExecutionUpdate, JobInvocation,
        JobInvocationOwner,
    };
    use batter::runledger::native::core::prelude::async_trait;
    use serde_json::json;
    use std::sync::{Arc, Mutex};
    use std::time::Instant;
    use uuid::Uuid;

    /// Test services with an owned invocation, as the execution rustdoc shows.
    struct TestServices {
        deadline: Instant,
        invocation: JobInvocationOwner,
    }

    #[async_trait]
    impl JobExecutionServices for TestServices {
        fn deadline(&self) -> Instant {
            self.deadline
        }
        fn remaining_budget(&self) -> Duration {
            self.deadline.saturating_duration_since(Instant::now())
        }
        async fn persist_progress(
            &self,
            _: JobExecutionUpdate<'_>,
        ) -> Result<(), JobExecutionError> {
            Ok(())
        }
        fn invocation(&self) -> Option<JobInvocation> {
            Some(self.invocation.invocation())
        }
    }

    /// Records every final-state write with the time the phase had left.
    #[derive(Default)]
    struct RecordingLedger {
        writes: Mutex<Vec<(Uuid, Duration)>>,
        fail: bool,
    }

    #[async_trait]
    impl NotifiedLedger for RecordingLedger {
        async fn mark_notified(
            &self,
            context: &OperationContext,
            id: Uuid,
        ) -> Result<(), crate::records::QueryError> {
            context.check().map_err(OperationError::Interrupted)?;
            self.writes.lock().unwrap().push((id, context.remaining()));
            if self.fail {
                return Err(OperationError::Failed(sqlx::Error::PoolClosed));
            }
            Ok(())
        }
    }

    fn context() -> JobContext {
        JobContext {
            job_id: Uuid::now_v7(),
            run_number: 1,
            attempt: 1,
            organization_id: None,
            worker_id: "test-worker".to_owned(),
            checkpoint: None,
        }
    }

    async fn execute(
        handler: &NotifyRecord<Arc<RecordingLedger>>,
        budget: Duration,
        payload: Value,
    ) -> Result<JobCompletion, JobFailure> {
        let services = TestServices {
            deadline: Instant::now() + budget,
            invocation: JobInvocationOwner::new(),
        };
        let context = context();
        let result = handler
            .execute(JobExecution::new(&context, &services), payload)
            .await;
        assert!(services.invocation.end().is_empty());
        result
    }

    #[tokio::test]
    async fn successful_provider_step_records_notified_at() {
        let ledger = Arc::new(RecordingLedger::default());
        let handler = NotifyRecord::new(
            ledger.clone(),
            Duration::from_millis(10),
            Duration::from_secs(1),
        );
        let id = Uuid::now_v7();
        let completion = execute(&handler, Duration::from_secs(5), json!({ "record_id": id }))
            .await
            .expect("provider completes within its limit");
        assert_eq!(completion.output(), None);
        let writes = ledger.writes.lock().unwrap();
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0].0, id);
    }

    #[tokio::test]
    async fn provider_timeout_still_records_notified_at_inside_the_reserve() {
        let ledger = Arc::new(RecordingLedger::default());
        // The provider takes longer than its limit; the invocation keeps a
        // 2 s budget so the final-state write still has time.
        let handler = NotifyRecord::new(
            ledger.clone(),
            Duration::from_secs(5),
            Duration::from_millis(50),
        );
        let id = Uuid::now_v7();
        let started = Instant::now();
        let failure = execute(&handler, Duration::from_secs(2), json!({ "record_id": id }))
            .await
            .expect_err("the provider step timed out");
        assert_eq!(failure.code, "records.notify_unknown");
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "the limit interrupted the sleep"
        );
        let writes = ledger.writes.lock().unwrap();
        assert_eq!(
            writes.len(),
            1,
            "the final-state write ran after the timeout"
        );
        assert_eq!(writes[0].0, id);
        assert!(
            writes[0].1 > Duration::from_millis(500),
            "finalization kept the reserve"
        );
    }

    #[tokio::test]
    async fn work_phase_expiry_leaves_the_reserve_for_the_write() {
        let ledger = Arc::new(RecordingLedger::default());
        // Provider limit exceeds the work budget: the child is clamped to the
        // work deadline (budget minus reserve), so the sleep is interrupted
        // and the write still sees roughly the reserve.
        let handler = NotifyRecord::new(
            ledger.clone(),
            Duration::from_secs(5),
            Duration::from_secs(5),
        );
        let failure = execute(
            &handler,
            Duration::from_millis(800),
            json!({ "record_id": Uuid::now_v7() }),
        )
        .await
        .expect_err("work expired before the provider finished");
        assert_eq!(failure.code, "records.notify_unknown");
        let writes = ledger.writes.lock().unwrap();
        assert_eq!(writes.len(), 1);
        assert!(writes[0].1 > Duration::from_millis(300));
        assert!(writes[0].1 <= Duration::from_millis(500));
    }

    #[tokio::test]
    async fn failed_final_state_write_is_retryable() {
        let ledger = Arc::new(RecordingLedger {
            fail: true,
            ..RecordingLedger::default()
        });
        let handler = NotifyRecord::new(
            ledger.clone(),
            Duration::from_millis(1),
            Duration::from_secs(1),
        );
        let failure = execute(
            &handler,
            Duration::from_secs(2),
            json!({ "record_id": Uuid::now_v7() }),
        )
        .await
        .expect_err("the write failed");
        assert_eq!(failure.code, "records.unrecorded");
    }

    #[tokio::test]
    async fn malformed_payload_is_terminal_and_writes_nothing() {
        let ledger = Arc::new(RecordingLedger::default());
        let handler = NotifyRecord::new(
            ledger.clone(),
            Duration::from_millis(1),
            Duration::from_secs(1),
        );
        let failure = execute(&handler, Duration::from_secs(2), json!({ "record": "x" }))
            .await
            .expect_err("payload lacks record_id");
        assert_eq!(failure.code, "records.invalid_payload");
        assert!(ledger.writes.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn exhausted_invocation_is_a_timeout_before_any_work() {
        let ledger = Arc::new(RecordingLedger::default());
        let handler = NotifyRecord::new(
            ledger.clone(),
            Duration::from_millis(1),
            Duration::from_secs(1),
        );
        // Less than the reserve remains, so no work time exists.
        let failure = execute(
            &handler,
            Duration::from_millis(100),
            json!({ "record_id": Uuid::now_v7() }),
        )
        .await
        .expect_err("no work time");
        assert_eq!(failure.code, "records.no_work_time");
        assert!(ledger.writes.lock().unwrap().is_empty());
    }

    #[test]
    fn catalog_registers_the_notify_handler() {
        let ledger = Arc::new(RecordingLedger::default());
        let catalog = catalog(ledger);
        let registry = catalog.to_registry();
        assert!(registry.get(NOTIFY_JOB).is_some());
    }
}
