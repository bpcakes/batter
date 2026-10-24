//! Progress persistence and declaration contracts of the worker-free handler
//! drive: the in-memory row merges like the queue row, refuses writes at the
//! deadline, starts from the declared checkpoint, and a budget is accepted
//! like any root budget. Exit and dispatch contracts are in `handler_invocation.rs`.

use batter_core::ConfigurationError;
use batter_runledger::test_support::{HandlerExit, HandlerInvocation, PersistedProgress};
use runledger_core::jobs::{
    JobCompletion, JobExecution, JobExecutionError, JobExecutionHandler, JobExecutionUpdate,
    JobFailure, JobProgressValidationError, JobType,
};
use runledger_core::prelude::async_trait;
use serde_json::{Value, json};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::advance;

const BUDGET: Duration = Duration::from_secs(10);

/// Commits progress in a fixed sequence and reports each write's result.
struct Writer(mpsc::UnboundedSender<Result<(), String>>);

impl Writer {
    async fn write(&self, execution: JobExecution<'_>, update: JobExecutionUpdate<'_>) {
        let result = execution
            .persist_progress(update)
            .await
            .map_err(|error| error.to_string());
        self.0.send(result).expect("test receives");
    }
}

#[async_trait]
impl JobExecutionHandler for Writer {
    fn job_type(&self) -> JobType<'static> {
        JobType::new("probe.writer")
    }

    async fn execute(
        &self,
        execution: JobExecution<'_>,
        payload: Value,
    ) -> Result<JobCompletion, JobFailure> {
        let resumed: Option<Value> = execution.checkpoint().expect("declared JSON");
        assert_eq!(resumed.as_ref(), execution.checkpoint_value());
        let update = |done, total| JobExecutionUpdate {
            progress_done: done,
            progress_total: total,
            checkpoint: None,
        };
        self.write(execution, update(Some(2), Some(10))).await;
        let result = execution.save_checkpoint(&json!({ "cursor": 2 })).await;
        self.0
            .send(result.map_err(|error| error.to_string()))
            .expect("test receives");
        // Retained total 10 rejects done 11; the update's own pair rejects -1.
        self.write(execution, update(Some(11), None)).await;
        self.write(execution, update(Some(-1), None)).await;
        if payload["mode"] == "expire" {
            advance(BUDGET).await;
            self.write(execution, update(Some(3), None)).await;
        }
        Ok(JobCompletion::success().checkpoint(resumed.unwrap_or(Value::Null)))
    }
}

fn writes(reports: &mut mpsc::UnboundedReceiver<Result<(), String>>) -> Vec<Result<(), String>> {
    std::iter::from_fn(|| reports.try_recv().ok()).collect()
}

#[tokio::test(start_paused = true)]
async fn progress_writes_merge_like_the_queue_row_and_stop_at_the_deadline() {
    let (report, mut reports) = mpsc::unbounded_channel();
    let handler = Writer(report).into_job_handler();
    let run = HandlerInvocation::new(BUDGET)
        .expect("budget")
        .with_checkpoint(json!({ "cursor": 0 }))
        .execute(&handler, json!({}))
        .await;
    let HandlerExit::Returned(Ok(completion)) = run.exit else {
        panic!("returned: {:?}", run.exit);
    };
    assert_eq!(
        completion.checkpoint_value(),
        Some(&json!({ "cursor": 0 })),
        "the handler read the declared checkpoint"
    );
    assert_eq!(
        writes(&mut reports),
        [
            Ok(()),
            Ok(()),
            Err(
                JobExecutionError::InvalidProgress(JobProgressValidationError::DoneExceedsTotal {
                    done: 11,
                    total: 10,
                })
                .to_string()
            ),
            Err(
                JobExecutionError::InvalidProgress(JobProgressValidationError::NegativeDone {
                    actual: -1
                })
                .to_string()
            ),
        ]
    );
    assert_eq!(
        run.persisted,
        PersistedProgress {
            progress_done: Some(2),
            progress_total: Some(10),
            checkpoint: Some(json!({ "cursor": 2 })),
        }
    );

    let expired = HandlerInvocation::new(BUDGET)
        .expect("budget")
        .execute(&handler, json!({ "mode": "expire" }))
        .await;
    assert_eq!(
        expired.exit,
        HandlerExit::TimedOut,
        "the result after the elapsed write is late"
    );
    assert_eq!(
        writes(&mut reports).last(),
        Some(&Err(JobExecutionError::DeadlineElapsed.to_string()))
    );
    assert_eq!(expired.persisted.progress_done, Some(2));
    assert_eq!(
        expired.persisted.checkpoint,
        Some(json!({ "cursor": 2 })),
        "an undeclared checkpoint starts absent and keeps the last commit"
    );
}

#[test]
fn the_budget_is_accepted_like_any_operation_budget() {
    assert!(matches!(
        HandlerInvocation::new(Duration::ZERO),
        Err(ConfigurationError::Zero(_))
    ));
    assert!(matches!(
        HandlerInvocation::new(Duration::from_secs(366 * 24 * 60 * 60)),
        Err(ConfigurationError::TooLarge(_))
    ));
    let first = HandlerInvocation::new(BUDGET).expect("budget");
    let second = HandlerInvocation::new(BUDGET).expect("budget");
    assert_ne!(
        first.context().job_id,
        second.context().job_id,
        "each declaration is its own job"
    );
    assert_eq!(
        (first.context().run_number, first.context().attempt),
        (1, 1)
    );
    assert_eq!(first.context().checkpoint, None);
    assert_eq!(
        first.with_checkpoint(json!(1)).context().checkpoint,
        Some(json!(1))
    );
}
