//! Inner contracts; typed decoding belongs after the gate.
use super::JobFlags;
use runledger_core::{
    jobs::{
        JobCompletion, JobContext, JobContract, JobDeadLetterInfo, JobExecution, JobFailure,
        JobType, malformed_job_payload,
    },
    prelude::async_trait,
};
use serde_json::Value;

/// A handler entered with one frozen view and native invocation services.
/// See the module example for construction through [`crate::Flags`].
#[async_trait]
pub trait FlagAwareHandler: Send + Sync {
    /// Stable identity for registration and routing, unchanged for its lifetime.
    fn job_type(&self) -> JobType<'static>;

    /// Execute under the view captured by the wrapper for this invocation.
    async fn execute(
        &self,
        execution: JobExecution<'_>,
        flags: &JobFlags,
        payload: Value,
    ) -> Result<JobCompletion, JobFailure>;

    /// Native dead-letter delivery, including the original undecoded payload.
    async fn on_dead_letter(
        &self,
        _context: JobContext,
        _payload: Value,
        _info: JobDeadLetterInfo,
    ) {
    }
}

/// Typed payload decoding after admission through the switch gate.
///
/// ```
/// use batter_flags::{Flags, jobs::{FlagAwareTypedHandler, JobFlags}};
/// use runledger_core::{prelude::async_trait, jobs::{JobContract, JobSpec, JobType,
///     JobExecution, JobCompletion, JobFailure}};
/// struct Number;
/// impl JobContract for Number {
///     type Payload = u32;
///     fn spec() -> JobSpec { JobSpec::new(JobType::new("example.number")).expect("static type") }
/// }
/// struct Handler;
/// #[async_trait]
/// impl FlagAwareTypedHandler for Handler {
///     type Contract = Number;
///     async fn execute(&self, _: JobExecution<'_>, _: &JobFlags, n: u32)
///         -> Result<JobCompletion, JobFailure> {
///         Ok(JobCompletion::with_output(n.into()))
///     }
/// }
/// fn wrap(flags: &Flags) {
///     let handler = flags.flagged(Handler.into_flag_aware());
///     # let _ = handler;
/// }
/// ```
#[async_trait]
pub trait FlagAwareTypedHandler: Send + Sync {
    /// Native durable payload contract.
    type Contract: JobContract;

    /// Execute the decoded payload under the invocation's frozen view.
    async fn execute(
        &self,
        execution: JobExecution<'_>,
        flags: &JobFlags,
        payload: <Self::Contract as JobContract>::Payload,
    ) -> Result<JobCompletion, JobFailure>;

    /// Classify decoding failure. The default never discloses payload details.
    /// The source can contain input data; sanitize any custom diagnostics.
    fn malformed_payload(&self, _source: &serde_json::Error) -> JobFailure {
        malformed_job_payload()
    }

    /// Receives raw JSON even when decoding failed.
    async fn on_dead_letter(
        &self,
        _context: JobContext,
        _payload: Value,
        _info: JobDeadLetterInfo,
    ) {
    }

    /// Adapt once, then pass to `Flags::flagged` or either gate constructor.
    fn into_flag_aware(self) -> TypedFlagAware<Self>
    where
        Self: Sized,
    {
        TypedFlagAware(self)
    }
}

/// Field-private typed adapter; constructed by
/// [`FlagAwareTypedHandler::into_flag_aware`].
pub struct TypedFlagAware<H>(H);

#[async_trait]
impl<H: FlagAwareTypedHandler> FlagAwareHandler for TypedFlagAware<H> {
    fn job_type(&self) -> JobType<'static> {
        H::Contract::spec().job_type()
    }

    async fn execute(
        &self,
        execution: JobExecution<'_>,
        flags: &JobFlags,
        payload: Value,
    ) -> Result<JobCompletion, JobFailure> {
        let payload =
            serde_json::from_value(payload).map_err(|error| self.0.malformed_payload(&error))?;
        self.0.execute(execution, flags, payload).await
    }

    async fn on_dead_letter(&self, context: JobContext, payload: Value, info: JobDeadLetterInfo) {
        self.0.on_dead_letter(context, payload, info).await;
    }
}
