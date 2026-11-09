//! Job gates and invocation-scoped decisions over native Runledger 0.13.
//!
//! Construct through [`Flags::flagged`], [`Flags::gated_direct`] or
//! [`Flags::gated_workflow_step`]. A gate checks before typed decoding and
//! captures the exact view handed to the handler. It never interrupts an
//! invocation already admitted. Register the wrapper in the native catalog
//! and run its supervisor through `batter_runledger::register_in`.
//!
//! ```
//! use batter_flags::{Flags, core::KillSwitch, jobs::{
//!     FlagAwareHandler, JobFlags, GatePolicy, DirectWhenTripped, Recheck,
//! }};
//! use runledger_core::{prelude::async_trait, jobs::{
//!     JobType, JobExecution, JobCompletion, JobFailure,
//! }};
//! use serde_json::Value;
//! use std::time::Duration;
//! struct Export;
//! #[async_trait]
//! impl FlagAwareHandler for Export {
//!     fn job_type(&self) -> JobType<'static> { JobType::new("example.export") }
//!     async fn execute(&self, _: JobExecution<'_>, _: &JobFlags, _: Value)
//!         -> Result<JobCompletion, JobFailure> { Ok(JobCompletion::success()) }
//! }
//! fn register(flags: &Flags, halt: &KillSwitch) -> Result<(), Box<dyn std::error::Error>> {
//!     let recheck = Recheck::after(Duration::from_secs(30))?;
//!     let handler = flags.gated_direct(Export, halt, GatePolicy {
//!         when_tripped: DirectWhenTripped::Defer(recheck), awaiting: recheck,
//!     })?;
//!     // Pass handler to the application's native JobCatalog registration.
//!     # let _ = handler;
//!     Ok(())
//! }
//! ```
//!
//! ## Durable effects
//!
//! The `jobs_live` worker/persistence cases passed locally on PostgreSQL 18 on
//! 2026-11-06: continuation timing/checkpoint/resource release and next claim,
//! retry exhaustion, drop delivery, workflow refusal and keyed pin identity.
//! Alarm firing and downstream workflow cascades remain source-read, unverified.
//!
//! | Policy | Native outcome |
//! | --- | --- |
//! | `DirectWhenTripped::Defer` | `continue_after`: committed continuation advances `run_number`, resets attempts, retains checkpoint and releases resource claims; native continuation alarms are source-read, unverified. |
//! | `DeferAttempt` (either policy) | Retryable `flags.job_type_deferred`, with a retry lower bound; consumes attempts and dead-letters on exhaustion. |
//! | `Drop` (either policy) | Terminal `flags.job_type_dropped`; native delivery invokes the inner dead-letter hook. |
//! | Awaiting first publication | Always `DeferAttempt` with the separately required awaiting interval, even for `Drop`. |
//!
//! Retry exhaustion and terminal failures on workflow steps propagate through
//! native workflow settlement. A direct-only declaration is an application
//! assertion: enqueueing it as a non-opted workflow step causes its first
//! `Defer` to dead-letter with `job.workflow_handler_continuation_not_enabled`.
//! No native queue, definition enablement, claim or workflow policy is changed.
//! An unavailable initial publication can exhaust attempts. Native retry
//! policy may delay a retry beyond this gate's lower bound. Scheduled jobs
//! still materialize while tripped, and may accumulate for release; every
//! recheck costs a claim plus a continuation or failure write set.
//!
//! Pins belong in the application's domain row, stored atomically with enqueue
//! and replayed on retry. `Pinned::same_decision` compares key and variant;
//! keyed enqueue compares the entire payload, including pin origin/version.
//! Re-evaluating even the same variant at a new version can therefore conflict.
//! `JobFlags` has no unrestricted live evaluation: unpinned application facts
//! require [`UnpinnedBecause`], and incompatible pins carry their own fallback.

mod handler;
mod policy;
mod view;

pub use handler::{FlagAwareHandler, FlagAwareTypedHandler, TypedFlagAware};
pub use policy::{DirectWhenTripped, GatePolicy, Recheck, RecheckError, StepWhenTripped};
pub use view::{JobFlags, UnpinnedBecause};

use crate::{Flags, GateError, flags::gate::SwitchGate, reader::WorkCapture};
use batter_flags_core::{KillSwitch, low_level::GateReading};
use runledger_core::{
    jobs::{
        JobCompletion, JobContext, JobDeadLetterInfo, JobExecution, JobFailure, JobHandler, JobType,
    },
    prelude::async_trait,
};
use serde_json::Value;

/// View-only wrapper, constructed solely by [`Flags::flagged`].
pub struct FlaggedHandler<H> {
    flags: Flags,
    inner: H,
    awaiting: Recheck,
}

/// Checked switch and view wrapper, constructed solely by the two gate methods
/// on [`Flags`]. Its native dead-letter callback forwards raw JSON unchanged.
pub struct GatedHandler<H> {
    gate: SwitchGate,
    inner: H,
    policy: GatePolicy<DirectWhenTripped>,
}

impl Flags {
    /// Capture one view for each invocation without gating the job type.
    ///
    /// Under `LoadPolicy::unready_until_loaded`, an invocation before the
    /// first publication is deferred by `awaiting`, as a gated job's is: the
    /// process has no state to run it on and does not serve its declared
    /// defaults. Under `start_on_defaults`, in declared mode and with test
    /// flags, every invocation runs.
    ///
    /// Migration: `flagged(inner)` ran such an invocation on the declared
    /// defaults while the process was still unready; pass the recheck to defer
    /// it with.
    pub fn flagged<H: FlagAwareHandler>(&self, inner: H, awaiting: Recheck) -> FlaggedHandler<H> {
        FlaggedHandler {
            flags: self.clone(),
            inner,
            awaiting,
        }
    }

    /// Gate an application-declared direct job type. Validates switch catalog
    /// and process ownership before registration; see the module example.
    pub fn gated_direct<H: FlagAwareHandler>(
        &self,
        inner: H,
        switch: &KillSwitch,
        policy: GatePolicy<DirectWhenTripped>,
    ) -> Result<GatedHandler<H>, GateError> {
        Ok(GatedHandler {
            gate: self.gate(switch)?,
            inner,
            policy,
        })
    }

    /// Gate a workflow-capable type; the policy cannot express continuation.
    ///
    /// ```compile_fail
    /// # use batter_flags::{Flags, core::KillSwitch, jobs::*};
    /// # fn invalid<H: FlagAwareHandler>(flags: &Flags, inner: H, switch: &KillSwitch, recheck: Recheck) {
    /// flags.gated_workflow_step(inner, switch, GatePolicy {
    ///     when_tripped: DirectWhenTripped::Defer(recheck), awaiting: recheck,
    /// });
    /// # }
    /// ```
    pub fn gated_workflow_step<H: FlagAwareHandler>(
        &self,
        inner: H,
        switch: &KillSwitch,
        policy: GatePolicy<StepWhenTripped>,
    ) -> Result<GatedHandler<H>, GateError> {
        self.gated_direct(
            inner,
            switch,
            GatePolicy {
                when_tripped: policy.when_tripped.into(),
                awaiting: policy.awaiting,
            },
        )
    }
}

fn services_required() -> JobFailure {
    JobFailure::terminal(
        "job.execution_services_required",
        "This handler requires runtime-owned execution services.",
    )
}

fn deferred(recheck: Recheck, context: &JobContext) -> JobFailure {
    JobFailure::retryable(
        "flags.job_type_deferred",
        "Job type is deferred by its flag gate.",
    )
    .retry_not_before_delay(recheck.delay(context))
}

#[async_trait]
impl<H: FlagAwareHandler> JobHandler for FlaggedHandler<H> {
    fn job_type(&self) -> JobType<'static> {
        self.inner.job_type()
    }
    async fn execute(&self, _: JobContext, _: Value) -> Result<JobCompletion, JobFailure> {
        Err(services_required())
    }
    async fn execute_with_services(
        &self,
        execution: JobExecution<'_>,
        payload: Value,
    ) -> Result<JobCompletion, JobFailure> {
        match self.flags.capture_for_work() {
            WorkCapture::Serve(view) => {
                self.inner
                    .execute(execution, &JobFlags(view), payload)
                    .await
            }
            WorkCapture::Unready => Err(deferred(self.awaiting, execution.context())),
        }
    }
    async fn on_dead_letter(&self, context: JobContext, payload: Value, info: JobDeadLetterInfo) {
        self.inner.on_dead_letter(context, payload, info).await;
    }
}

#[async_trait]
impl<H: FlagAwareHandler> JobHandler for GatedHandler<H> {
    fn job_type(&self) -> JobType<'static> {
        self.inner.job_type()
    }
    async fn execute(&self, _: JobContext, _: Value) -> Result<JobCompletion, JobFailure> {
        Err(services_required())
    }
    async fn execute_with_services(
        &self,
        execution: JobExecution<'_>,
        payload: Value,
    ) -> Result<JobCompletion, JobFailure> {
        let entered = self.gate.enter();
        match entered.reading {
            GateReading::Open(_) => {
                self.inner
                    .execute(execution, &JobFlags(entered.view), payload)
                    .await
            }
            GateReading::AwaitingPublication(_) => {
                Err(deferred(self.policy.awaiting, execution.context()))
            }
            GateReading::Tripped(_) => match self.policy.when_tripped {
                DirectWhenTripped::Defer(recheck) => Ok(JobCompletion::continue_after(
                    recheck.delay(execution.context()),
                )),
                DirectWhenTripped::DeferAttempt(recheck) => {
                    Err(deferred(recheck, execution.context()))
                }
                DirectWhenTripped::Drop => Err(JobFailure::terminal(
                    "flags.job_type_dropped",
                    "Job type was dropped by its flag gate.",
                )),
            },
        }
    }
    async fn on_dead_letter(&self, context: JobContext, payload: Value, info: JobDeadLetterInfo) {
        self.inner.on_dead_letter(context, payload, info).await;
    }
}

#[cfg(test)]
mod tests;
