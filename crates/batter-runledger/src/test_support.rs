//! Drive one job handler invocation in a test, without a worker, queue or
//! database.
//!
//! [`HandlerInvocation`] builds the real [`JobExecution`] a handler receives
//! from the Runledger worker: services that own the invocation's exit signal,
//! one monotonic deadline a declared budget after execution starts, and
//! in-memory progress persistence with the queue row's merge rule. It drives
//! the handler through [`JobHandler::execute_with_services`], the worker's own
//! dispatch, destroys the handler future, ends the invocation and returns an
//! exhaustive [`HandlerExit`]. Nothing here schedules retries, continuations
//! or dead letters: durable outcome policy stays in the worker, and the
//! returned exit is the evidence that policy would be applied to.
//!
//! Selected by this crate's `test-support` feature, so it is reachable as
//! `batter::runledger::test_support` with the facade's `runledger-test-support`
//! feature. Like `native::test_support`, it belongs in `[dev-dependencies]`.

use batter_core::{ConfigurationError, operation::RootDeadline};
use runledger_core::jobs::{
    JobCompletion, JobContext, JobExecution, JobExecutionError, JobExecutionServices,
    JobExecutionUpdate, JobFailure, JobHandler, JobInvocation, JobInvocationHookPanics,
    JobInvocationOwner, validate_job_progress,
};
use runledger_core::prelude::async_trait;
use serde_json::Value;
use std::{
    any::Any,
    future::Future,
    panic::{AssertUnwindSafe, catch_unwind},
    pin::Pin,
    sync::{Mutex, PoisonError},
    task::{Context, Poll},
    time::Duration,
};
use tokio::time::Instant;
use uuid::Uuid;

/// Lease owner recorded in the context of every invocation driven here.
const WORKER_ID: &str = "test-support";

/// One handler invocation declared for a test: a budget and a resume snapshot.
///
/// [`Self::execute`] is the whole drive. It creates one [`JobInvocationOwner`]
/// for the invocation, so [`JobExecution::invocation`] is always `Some` and
/// [`crate::job_phases`] never returns [`crate::JobPhasesRejection::Unsupported`];
/// it sets the native deadline `budget` after the returned future is first
/// polled and enforces it as the worker does; it destroys the handler future
/// before it ends the invocation, on every exit and when the future itself is
/// destroyed; and it returns the contained exit-hook panics instead of
/// dropping them. No end, drop or join obligation is left with the test.
///
/// Pass the value a catalog registers: `handler.into_job_handler()` for a
/// [`runledger_core::jobs::JobExecutionHandler`] or
/// [`runledger_core::jobs::TypedJobHandler`]. The handler is dispatched through
/// [`JobHandler::execute_with_services`], so an adapted handler is never asked
/// to run without services.
///
/// ```
/// use batter_core::operation::OperationError;
/// use batter_runledger::test_support::{HandlerExit, HandlerInvocation};
/// use batter_runledger::{JobPhasesRejection, job_phases};
/// use runledger_core::jobs::{
///     JobCompletion, JobExecution, JobExecutionHandler, JobFailure, JobType,
/// };
/// use runledger_core::prelude::async_trait;
/// use serde_json::{Value, json};
/// use std::time::Duration;
///
/// const RECORD_RESERVE: Duration = Duration::from_millis(300);
///
/// struct Notify;
///
/// #[async_trait]
/// impl JobExecutionHandler for Notify {
///     fn job_type(&self) -> JobType<'static> {
///         JobType::new("example.notify")
///     }
///
///     async fn execute(
///         &self,
///         execution: JobExecution<'_>,
///         payload: Value,
///     ) -> Result<JobCompletion, JobFailure> {
///         let phases = job_phases(execution, RECORD_RESERVE).map_err(|rejection| {
///             match rejection {
///                 JobPhasesRejection::Exhausted | JobPhasesRejection::Ended => {
///                     JobFailure::timeout("example.no_work_time", "No work time remained.")
///                 }
///                 JobPhasesRejection::Unsupported | JobPhasesRejection::Reserve(_) => {
///                     JobFailure::terminal("example.unsupported", "Invocation phases unavailable.")
///                 }
///             }
///         })?;
///         let accepted = payload.get("recipient").is_some();
///         let sent = phases
///             .work()
///             .run("example.send", |_scope| async move {
///                 accepted.then_some("receipt").ok_or(std::io::ErrorKind::InvalidInput)
///             })
///             .await;
///         let outcome = match sent {
///             Ok(receipt) => Ok(JobCompletion::with_output(json!({ "receipt": receipt }))),
///             Err(OperationError::Failed(_)) => {
///                 Err(JobFailure::terminal("example.invalid_payload", "No recipient."))
///             }
///             Err(OperationError::Interrupted(_)) => {
///                 Err(JobFailure::timeout("example.unknown", "Delivery is unknown."))
///             }
///         };
///         phases
///             .finalization()
///             .run("example.record", |_scope| async { Ok::<_, std::io::Error>(()) })
///             .await
///             .map_err(|_| JobFailure::retryable("example.unrecorded", "Not recorded."))?;
///         outcome
///     }
/// }
///
/// # tokio::runtime::Builder::new_current_thread().enable_time().start_paused(true).build()?.block_on(async {
/// let handler = Notify.into_job_handler();
/// let run = HandlerInvocation::new(Duration::from_secs(5))?
///     .execute(&handler, json!({ "recipient": "a" }))
///     .await;
/// let HandlerExit::Returned(Ok(completion)) = run.exit else {
///     panic!("delivered: {:?}", run.exit);
/// };
/// assert_eq!(completion.output(), Some(&json!({ "receipt": "receipt" })));
/// assert!(run.exit_hook_panics.is_empty());
///
/// let run = HandlerInvocation::new(Duration::from_secs(5))?
///     .execute(&handler, json!({}))
///     .await;
/// assert!(matches!(
///     run.exit,
///     HandlerExit::Returned(Err(failure)) if failure.code == "example.invalid_payload"
/// ));
///
/// // Exactly the reserve leaves no work time: the handler's own mapping answers.
/// let run = HandlerInvocation::new(RECORD_RESERVE)?
///     .execute(&handler, json!({ "recipient": "a" }))
///     .await;
/// assert!(matches!(
///     run.exit,
///     HandlerExit::Returned(Err(failure)) if failure.code == "example.no_work_time"
/// ));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// # })?;
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
///
/// Nothing a handler or test reaches can end the invocation early:
///
/// ```compile_fail,E0616
/// fn cannot_reach_the_owner(invocation: batter_runledger::test_support::HandlerInvocation) {
///     let _ = invocation.budget;
/// }
/// ```
#[derive(Debug)]
#[must_use = "a declared invocation runs nothing until it is executed"]
pub struct HandlerInvocation {
    budget: Duration,
    context: JobContext,
}

impl HandlerInvocation {
    /// Declare one invocation whose native deadline is `budget` after
    /// [`Self::execute`] is first polled.
    ///
    /// The budget is accepted as any operation budget is: zero and durations
    /// longer than one year are rejected here, before any handler runs.
    pub fn new(budget: Duration) -> Result<Self, ConfigurationError> {
        // The same acceptance as every Batter root budget. The instant is not
        // kept: execution, not declaration, starts the clock.
        RootDeadline::after(budget)?;
        Ok(Self {
            budget,
            context: JobContext {
                job_id: Uuid::now_v7(),
                run_number: 1,
                attempt: 1,
                organization_id: None,
                worker_id: WORKER_ID.to_owned(),
                checkpoint: None,
            },
        })
    }

    /// Resume from a durable checkpoint, as a later run or attempt would.
    ///
    /// The handler reads it through [`JobExecution::checkpoint`], and it stays
    /// the persisted checkpoint until the handler commits another.
    pub fn with_checkpoint(mut self, checkpoint: Value) -> Self {
        self.context.checkpoint = Some(checkpoint);
        self
    }

    /// The context the handler will receive: a fresh job identity, run 1,
    /// attempt 1, no organization and the declared checkpoint.
    #[must_use]
    pub fn context(&self) -> &JobContext {
        &self.context
    }

    /// Drive `handler` once to its exit.
    ///
    /// The exit is classified as the worker classifies it: a result or panic
    /// observed strictly before the deadline is returned as such, and anything
    /// later, including a handler that never yields, is [`HandlerExit::TimedOut`].
    /// Destroying the returned future before it completes ends the invocation
    /// exactly as a worker task abort does: the handler future is dropped
    /// first, so work awaited inside it stops without observing cancellation,
    /// while spawned work holding a derived context is cancelled.
    pub async fn execute<H: JobHandler + ?Sized>(self, handler: &H, payload: Value) -> HandlerRun {
        let Self { budget, context } = self;
        // Declared in the worker's order: destroying this future drops the
        // handler future before the owner ends the invocation.
        let owner = JobInvocationOwner::new();
        let services = Services {
            deadline: Instant::now() + budget,
            invocation: owner.invocation(),
            persisted: Mutex::new(PersistedProgress {
                progress_done: None,
                progress_total: None,
                checkpoint: context.checkpoint.clone(),
            }),
        };
        let exit = {
            let execution = JobExecution::new(&context, &services);
            let handler = CatchUnwind(handler.execute_with_services(execution, payload));
            match tokio::time::timeout_at(services.deadline, handler).await {
                // Deadline precedence does not depend on which future the
                // timeout polled first.
                Ok(observed) if Instant::now() < services.deadline => match observed {
                    Ok(returned) => HandlerExit::Returned(returned),
                    Err(payload) => HandlerExit::Panicked(panic_message(&*payload)),
                },
                _ => HandlerExit::TimedOut,
            }
        };
        let exit_hook_panics = owner.end();
        HandlerRun {
            exit,
            persisted: services.into_persisted(),
            exit_hook_panics,
        }
    }
}

/// Everything one driven invocation left behind.
#[derive(Debug)]
#[must_use = "the exit, persisted progress and contained hook panics are the test's evidence"]
#[non_exhaustive]
pub struct HandlerRun {
    /// How the invocation ended.
    pub exit: HandlerExit,
    /// The durable progress and checkpoint after the last committed write, or
    /// the declared checkpoint when the handler committed none.
    pub persisted: PersistedProgress,
    /// Waiter-wake and exit-hook panics contained when the invocation ended.
    /// The worker retains these as callback evidence; a test should assert
    /// that they are empty rather than drop them.
    pub exit_hook_panics: JobInvocationHookPanics,
}

/// How a driven handler invocation ended, as the worker would observe it.
///
/// Lease loss is not representable here: there is no lease.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use = "an exit that is not inspected proves nothing about the handler"]
pub enum HandlerExit {
    /// The handler returned this result and it was observed strictly before
    /// the deadline. The worker applies its durable outcome policy to it;
    /// this facility does not.
    Returned(Result<JobCompletion, JobFailure>),
    /// No result was observed strictly before the deadline. The handler
    /// future was destroyed; the worker records `job.timeout_exceeded`.
    TimedOut,
    /// The handler panicked before the deadline, with this message. The worker
    /// records `job.handler_panic`.
    Panicked(String),
}

/// The durable progress fields of the job row after the invocation.
///
/// Each committed [`JobExecution::persist_progress`] replaced the fields it
/// supplied and retained the others, as the queue row does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistedProgress {
    /// Completed work, when a write supplied it.
    pub progress_done: Option<i64>,
    /// Total work, when a write supplied it.
    pub progress_total: Option<i64>,
    /// The latest committed checkpoint, starting from the declared one.
    pub checkpoint: Option<Value>,
}

/// Services for one invocation: the worker's deadline and exit claim, with the
/// queue row replaced by memory.
struct Services {
    deadline: Instant,
    invocation: JobInvocation,
    persisted: Mutex<PersistedProgress>,
}

impl Services {
    fn into_persisted(self) -> PersistedProgress {
        self.persisted
            .into_inner()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

#[async_trait]
impl JobExecutionServices for Services {
    fn deadline(&self) -> std::time::Instant {
        self.deadline.into_std()
    }

    fn remaining_budget(&self) -> Duration {
        self.deadline.saturating_duration_since(Instant::now())
    }

    fn invocation(&self) -> Option<JobInvocation> {
        Some(self.invocation.clone())
    }

    async fn persist_progress(
        &self,
        update: JobExecutionUpdate<'_>,
    ) -> Result<(), JobExecutionError> {
        // The worker refuses writes at or after the deadline and never extends it.
        if Instant::now() >= self.deadline {
            return Err(JobExecutionError::DeadlineElapsed);
        }
        let mut persisted = self
            .persisted
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        // Omitted fields retain their durable values, and the effective pair
        // is validated as the row constraint validates it.
        let progress_done = update.progress_done.or(persisted.progress_done);
        let progress_total = update.progress_total.or(persisted.progress_total);
        validate_job_progress(progress_done, progress_total)
            .map_err(JobExecutionError::InvalidProgress)?;
        persisted.progress_done = progress_done;
        persisted.progress_total = progress_total;
        if let Some(checkpoint) = update.checkpoint {
            persisted.checkpoint = Some(checkpoint.clone());
        }
        Ok(())
    }
}

/// Contain a handler panic at the poll boundary, as the worker does.
struct CatchUnwind<F>(F);

impl<F: Future + Unpin> Future for CatchUnwind<F> {
    type Output = Result<F::Output, Box<dyn Any + Send>>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match catch_unwind(AssertUnwindSafe(|| Pin::new(&mut self.0).poll(cx))) {
            Ok(Poll::Pending) => Poll::Pending,
            Ok(Poll::Ready(output)) => Poll::Ready(Ok(output)),
            Err(payload) => Poll::Ready(Err(payload)),
        }
    }
}

fn panic_message(payload: &(dyn Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<String>() {
        return message.clone();
    }
    if let Some(message) = payload.downcast_ref::<&'static str>() {
        return (*message).to_owned();
    }
    "non-string panic payload".to_owned()
}
