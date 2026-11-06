//! Protected work/finalization sequencing inside a native handler invocation.
use crate::{JobPhasesRejection, job_phases};
use batter_core::{
    ConfigurationError,
    operation::{
        OperationContext, PhaseOutcome, PhasedFailure,
        low_level::{ScopedPhases, drive_phases},
    },
};
use runledger_core::{
    jobs::{
        JobCompletion, JobContext, JobDeadLetterInfo, JobExecution, JobFailure, JobHandler, JobType,
    },
    prelude::async_trait,
};
use serde_json::Value;
use std::{fmt, time::Duration};

/// Work-phase access to borrowed native services and the shortened budget.
/// Neither finalization nor its context is available here.
pub struct JobWork<'a> {
    execution: JobExecution<'a>,
    context: &'a OperationContext,
}
impl JobWork<'_> {
    /// Native progress/checkpoint services, retaining their lease fencing.
    pub fn execution(&self) -> JobExecution<'_> {
        self.execution
    }
    /// Work deadline and cancellation lineage derived from this invocation.
    pub fn context(&self) -> &OperationContext {
        self.context
    }
}

/// Finalization access, created only after work-future destruction.
pub struct JobFinalization<'a, T, E> {
    execution: JobExecution<'a>,
    context: &'a OperationContext,
    work: &'a PhaseOutcome<T, E>,
}
impl<T, E> JobFinalization<'_, T, E> {
    /// The same native services used by work; no new lease or deadline.
    pub fn execution(&self) -> JobExecution<'_> {
        self.execution
    }
    /// The original native deadline, including the reserved time.
    pub fn context(&self) -> &OperationContext {
        self.context
    }
    /// Complete work evidence, including panic and interrupted-return evidence.
    pub fn work(&self) -> &PhaseOutcome<T, E> {
        self.work
    }
}

/// Complete evidence presented to application failure classification.
/// A failure classifier must return a native failure, never a successful job.
#[must_use = "classify the complete invocation failure"]
pub enum PhasedJobFailure<T, E> {
    /// No phase ran because the invocation could not supply valid phases.
    Rejected(JobPhasesRejection),
    /// At least one phase failed; both outcomes remain available.
    Phases(PhasedFailure<T, E, JobCompletion, E>),
}
impl<T, E> fmt::Debug for PhasedJobFailure<T, E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Rejected(error) => f.debug_tuple("Rejected").field(error).finish(),
            Self::Phases(error) => f.debug_tuple("Phases").field(error).finish(),
        }
    }
}

/// Application phases registered through [`PhasedJob`]. Both callbacks may
/// borrow `self` and native execution services across awaits.
///
/// The adapter owns ordering, phase deadlines, child cancellation, destruction
/// and failure retention. An early `?` or panic in work still reaches finalization
/// while the invocation remains live. The final completion is accepted only
/// when both phases succeed. Application error/retry policy stays in `failed`.
#[async_trait]
pub trait PhasedJobHandler: Send + Sync {
    /// Provisional work output, available to finalization by reference.
    type Output: Send + Sync;
    /// Concrete application failure, retained separately for both phases.
    type Error: Send + Sync;
    /// Stable native job identity.
    fn job_type(&self) -> JobType<'static>;
    /// Perform work under its shortened deadline.
    async fn work(&self, scope: JobWork<'_>, payload: Value) -> Result<Self::Output, Self::Error>;
    /// Record the outcome using the reserve and return the proposed completion.
    async fn finalize(
        &self,
        scope: JobFinalization<'_, Self::Output, Self::Error>,
    ) -> Result<JobCompletion, Self::Error>;
    /// Classify all retained failure evidence into native job policy.
    fn failed(&self, failure: PhasedJobFailure<Self::Output, Self::Error>) -> JobFailure;
    /// Native dead-letter callback, outside the work/finalization phases.
    async fn on_dead_letter(
        &self,
        _context: JobContext,
        _payload: Value,
        _info: JobDeadLetterInfo,
    ) {
    }
}

/// A native handler whose work/finalization sequence is owned by the adapter.
/// Register this value directly with the native registry or catalog.
///
/// No separately cancellable phase runner is exposed on the consumer path.
/// The native worker owns the entire invocation. Lease loss, the native deadline
/// or task destruction ends that authority and may interrupt finalization; this
/// does not promise asynchronous Drop, detached-task joins or remote rollback.
/// It cannot run finalization after native invocation death. Custom runtimes
/// must obey `JobHandler::execute_with_services`'s ownership contract.
///
/// ```
/// use batter_runledger::{PhasedJob, PhasedJobHandler, PhasedJobFailure, JobWork, JobFinalization};
/// use runledger_core::{jobs::{JobType, JobCompletion, JobFailure, JobExecutionError}, prelude::async_trait};
/// use serde_json::Value;
/// use std::time::Duration;
/// struct Count;
/// #[async_trait]
/// impl PhasedJobHandler for Count {
///     type Output = u64;
///     type Error = JobExecutionError;
///     fn job_type(&self) -> JobType<'static> { JobType::new("example.count") }
///     async fn work(&self, scope: JobWork<'_>, _: Value) -> Result<u64, Self::Error> {
///         scope.execution().save_checkpoint(&1_u64).await?;
///         Ok(2)
///     }
///     async fn finalize(&self, scope: JobFinalization<'_, u64, Self::Error>)
///         -> Result<JobCompletion, Self::Error> {
///         if let Ok(count) = scope.work().classify() {
///             scope.execution().save_checkpoint(count).await?;
///         }
///         Ok(JobCompletion::success())
///     }
///     fn failed(&self, _: PhasedJobFailure<u64, Self::Error>) -> JobFailure {
///         JobFailure::terminal("example.count_failed", "Count phases failed.")
///     }
/// }
/// let handler = PhasedJob::new(Count, Duration::from_millis(200))?;
/// # let _ = handler;
/// # Ok::<(), batter_core::ConfigurationError>(())
/// ```
/// Work cannot access finalization or forge its scope:
/// ```compile_fail,E0599
/// fn early(work: batter_runledger::JobWork<'_>) { work.finalization(); }
/// ```
/// ```compile_fail,E0451
/// fn forge(execution: runledger_core::jobs::JobExecution<'_>,
///     context: &batter_core::operation::OperationContext) {
///     let _ = batter_runledger::JobWork { execution, context };
/// }
/// ```
pub struct PhasedJob<H> {
    handler: H,
    reserve: Duration,
}
impl<H: PhasedJobHandler> PhasedJob<H> {
    /// Validate a positive reserve before registration. Per-invocation checks
    /// reject deadlines that leave no work time before any callback runs.
    pub fn new(handler: H, reserve: Duration) -> Result<Self, ConfigurationError> {
        if reserve.is_zero() {
            return Err(ConfigurationError::Zero("job finalization reserve"));
        }
        Ok(Self { handler, reserve })
    }
}

#[async_trait]
impl<H: PhasedJobHandler> JobHandler for PhasedJob<H> {
    fn job_type(&self) -> JobType<'static> {
        self.handler.job_type()
    }
    async fn execute(&self, _: JobContext, _: Value) -> Result<JobCompletion, JobFailure> {
        Err(JobFailure::terminal(
            "job.execution_services_required",
            "This handler requires runtime-owned execution services.",
        ))
    }
    async fn execute_with_services(
        &self,
        execution: JobExecution<'_>,
        payload: Value,
    ) -> Result<JobCompletion, JobFailure> {
        batter_core::telemetry::with_current_dispatch(async {
            let phases = job_phases(execution, self.reserve)
                .map_err(|error| self.handler.failed(PhasedJobFailure::Rejected(error)))?;
            let mut driver = Driver {
                handler: &self.handler,
                execution,
                payload: Some(payload),
            };
            match drive_phases(phases, &mut driver).await {
                Ok((_work, completion)) => Ok(completion),
                Err(error) => Err(self.handler.failed(PhasedJobFailure::Phases(error))),
            }
        })
        .await
    }
    async fn on_dead_letter(&self, context: JobContext, payload: Value, info: JobDeadLetterInfo) {
        self.handler.on_dead_letter(context, payload, info).await;
    }
}

struct Driver<'a, H> {
    handler: &'a H,
    execution: JobExecution<'a>,
    payload: Option<Value>,
}
impl<H: PhasedJobHandler> ScopedPhases for Driver<'_, H> {
    type Work = H::Output;
    type WorkError = H::Error;
    type Final = JobCompletion;
    type FinalError = H::Error;
    async fn work(&mut self, context: &OperationContext) -> Result<H::Output, H::Error> {
        self.handler
            .work(
                JobWork {
                    execution: self.execution,
                    context,
                },
                self.payload.take().expect("phase driver invokes work once"),
            )
            .await
    }
    async fn finalize(
        &mut self,
        context: &OperationContext,
        work: &PhaseOutcome<H::Output, H::Error>,
    ) -> Result<JobCompletion, H::Error> {
        self.handler
            .finalize(JobFinalization {
                execution: self.execution,
                context,
                work,
            })
            .await
    }
}
