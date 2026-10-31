//! Progress persistence and declaration contracts of the worker-free handler
//! drive: the in-memory row merges like the queue row, refuses writes at the
//! deadline, starts from the declared checkpoint and retained progress, which
//! bounds a resumed attempt's writes, a budget is accepted like any root
//! budget, and a claimed context reaches the handler unchanged once its run
//! and attempt are positive. Exit and dispatch contracts are in
//! `handler_invocation.rs`.

use batter_core::ConfigurationError;
use batter_runledger::test_support::{HandlerExit, HandlerInvocation, PersistedProgress};
use runledger_core::jobs::{
    JobCompletion, JobContext, JobExecution, JobExecutionError, JobExecutionHandler,
    JobExecutionUpdate, JobFailure, JobProgressValidationError, JobType,
};
use runledger_core::prelude::async_trait;
use serde_json::{Value, json};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::advance;
use uuid::Uuid;

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
        // A progress-only write keeps the committed checkpoint and total.
        self.write(execution, update(Some(4), None)).await;
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
            progress_done: Some(4),
            progress_total: Some(10),
            checkpoint: Some(json!({ "cursor": 2 })),
        },
        "the progress-only write replaced done and retained the total and checkpoint"
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
    assert_eq!(
        expired.persisted.progress_done,
        Some(4),
        "the write refused at the deadline committed nothing"
    );
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

/// Reports the context it received.
struct Identity;

#[async_trait]
impl JobExecutionHandler for Identity {
    fn job_type(&self) -> JobType<'static> {
        JobType::new("probe.identity")
    }

    async fn execute(
        &self,
        execution: JobExecution<'_>,
        _payload: Value,
    ) -> Result<JobCompletion, JobFailure> {
        let context = serde_json::to_value(execution.context()).expect("context is JSON");
        Ok(JobCompletion::with_output(context))
    }
}

fn claimed() -> JobContext {
    JobContext {
        job_id: Uuid::from_u128(1),
        run_number: 2,
        attempt: 3,
        organization_id: Some(Uuid::from_u128(4)),
        worker_id: "probe-worker".to_owned(),
        checkpoint: Some(json!({ "cursor": 5 })),
    }
}

#[tokio::test(start_paused = true)]
async fn a_claimed_context_reaches_the_handler_unchanged() {
    let invocation = HandlerInvocation::for_context(BUDGET, claimed()).expect("claimed");
    assert_eq!(invocation.context(), &claimed());
    let run = invocation
        .execute(&Identity.into_job_handler(), json!({}))
        .await;
    let HandlerExit::Returned(Ok(completion)) = run.exit else {
        panic!("identity: {:?}", run.exit);
    };
    assert_eq!(
        completion.output(),
        Some(&serde_json::to_value(claimed()).expect("context is JSON"))
    );
    assert_eq!(
        run.persisted.checkpoint,
        claimed().checkpoint,
        "the claimed checkpoint is the durable one until a commit"
    );
    assert!(run.exit_hook_panics.is_empty());

    let resumed = HandlerInvocation::for_context(BUDGET, claimed())
        .expect("claimed")
        .with_checkpoint(json!({ "cursor": 6 }));
    assert_eq!(
        resumed.context(),
        &JobContext {
            checkpoint: Some(json!({ "cursor": 6 })),
            ..claimed()
        },
        "a later checkpoint replaces only the checkpoint"
    );
}

#[test]
fn a_claimed_context_needs_a_positive_run_and_attempt() {
    for (run_number, attempt, field) in [
        (0, 1, "job run number"),
        (-1, 1, "job run number"),
        (1, 0, "job attempt"),
        (1, -1, "job attempt"),
    ] {
        let context = JobContext {
            run_number,
            attempt,
            ..claimed()
        };
        assert_eq!(
            HandlerInvocation::for_context(BUDGET, context).map(|_| ()),
            Err(ConfigurationError::Zero(field)),
            "run {run_number}, attempt {attempt}"
        );
    }
    assert!(matches!(
        HandlerInvocation::for_context(Duration::ZERO, claimed()),
        Err(ConfigurationError::Zero(_))
    ));
    let first = JobContext {
        run_number: 1,
        attempt: 1,
        ..claimed()
    };
    assert!(HandlerInvocation::for_context(BUDGET, first).is_ok());
}

/// Writes the completed count in its payload without a total, as an attempt
/// counting on from its checkpoint would, and reports the write's result.
struct CountOn(mpsc::UnboundedSender<Result<(), String>>);

#[async_trait]
impl JobExecutionHandler for CountOn {
    fn job_type(&self) -> JobType<'static> {
        JobType::new("probe.count_on")
    }

    async fn execute(
        &self,
        execution: JobExecution<'_>,
        payload: Value,
    ) -> Result<JobCompletion, JobFailure> {
        let update = JobExecutionUpdate {
            progress_done: payload["done"].as_i64(),
            progress_total: None,
            checkpoint: None,
        };
        let result = execution
            .persist_progress(update)
            .await
            .map_err(|error| error.to_string());
        self.0.send(result).expect("test receives");
        Ok(JobCompletion::success())
    }
}

#[tokio::test(start_paused = true)]
async fn a_resumed_attempt_writes_against_the_retained_progress() {
    let (report, mut reports) = mpsc::unbounded_channel();
    let handler = CountOn(report).into_job_handler();
    // An earlier attempt committed 4 of 10 and the claimed cursor; the retry
    // claimed the row with both.
    let retry = || {
        HandlerInvocation::for_context(BUDGET, claimed())
            .expect("claimed")
            .with_progress(Some(4), Some(10))
            .expect("the row's own pair")
    };
    let retained = PersistedProgress {
        progress_done: Some(4),
        progress_total: Some(10),
        checkpoint: claimed().checkpoint,
    };

    let refused = retry().execute(&handler, json!({ "done": 11 })).await;
    assert!(
        matches!(refused.exit, HandlerExit::Returned(Ok(_))),
        "{:?}",
        refused.exit
    );
    assert_eq!(
        writes(&mut reports),
        [Err(JobExecutionError::InvalidProgress(
            JobProgressValidationError::DoneExceedsTotal {
                done: 11,
                total: 10,
            }
        )
        .to_string())],
        "the retained total bounds a write that omits it"
    );
    assert_eq!(
        refused.persisted, retained,
        "the refused write committed nothing"
    );

    let counted = retry().execute(&handler, json!({ "done": 5 })).await;
    assert_eq!(writes(&mut reports), [Ok(())]);
    assert_eq!(
        counted.persisted,
        PersistedProgress {
            progress_done: Some(5),
            ..retained.clone()
        },
        "the write replaced done and kept the retained total"
    );

    // Declared after a later checkpoint, the progress replaces only its own
    // fields.
    let untouched = HandlerInvocation::for_context(BUDGET, claimed())
        .expect("claimed")
        .with_checkpoint(json!({ "cursor": 6 }))
        .with_progress(Some(4), Some(10))
        .expect("the row's own pair")
        .execute(&Identity.into_job_handler(), json!({}))
        .await;
    assert!(untouched.exit_hook_panics.is_empty());
    assert_eq!(
        untouched.persisted,
        PersistedProgress {
            checkpoint: Some(json!({ "cursor": 6 })),
            ..retained
        },
        "without a write the retained row is the durable one"
    );
}

#[test]
fn retained_progress_is_refused_where_the_row_refuses_it() {
    let declare = |done, total| {
        HandlerInvocation::new(BUDGET)
            .expect("budget")
            .with_progress(done, total)
            .map(|_| ())
    };
    assert_eq!(
        declare(Some(-1), None),
        Err(JobProgressValidationError::NegativeDone { actual: -1 })
    );
    assert_eq!(
        declare(None, Some(-1)),
        Err(JobProgressValidationError::NegativeTotal { actual: -1 })
    );
    assert_eq!(
        declare(Some(11), Some(10)),
        Err(JobProgressValidationError::DoneExceedsTotal {
            done: 11,
            total: 10
        })
    );
    for (done, total) in [
        (Some(10), Some(10)),
        (Some(3), None),
        (None, Some(0)),
        (None, None),
    ] {
        assert_eq!(declare(done, total), Ok(()), "{done:?} of {total:?}");
    }
}
