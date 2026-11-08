//! The `receipt.send` job: gated by `checkout_halt`, acting on the
//! `express_receipt` decision pinned when its checkout was accepted.
//!
//! While the halt is engaged the gate answers every claimed receipt with a
//! native continuation (`DirectWhenTripped::Defer`): the job is re-run after
//! the recheck interval with its attempt count reset, so a halt of any length
//! neither consumes attempts nor drops a receipt. Before the first flags
//! publication the library always defers an *attempt* instead (`awaiting`,
//! 30 to 37.5 s each), whatever `when_tripped` says. The last-good file makes
//! that window the first-boot case, and `MAX_ATTEMPTS` covers about a quarter
//! of an hour of it; a longer first-boot outage of the flags store would
//! dead-letter receipts with `flags.job_type_deferred`. That residual risk is
//! the library's fixed policy, not something this handler can choose.

use std::time::Duration;

use batter::{
    flags::{
        Flags, GateError,
        core::{Pinned, PinnedResolution, ReleaseFlag, Toggle},
        jobs::{
            DirectWhenTripped, FlagAwareTypedHandler, GatePolicy, GatedHandler, JobFlags, Recheck,
            TypedFlagAware,
        },
    },
    operation::{OperationContext, OperationError},
    runledger::{
        JobPhasesRejection, job_phases,
        native::{
            core::{
                jobs::{JobCompletion, JobContract, JobExecution, JobFailure, JobSpec, JobType},
                prelude::async_trait,
            },
            runtime::catalog::{CatalogError, JobCatalog, JobCatalogDefinitionOverrides},
        },
    },
};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

use crate::{flags::Handles, orders};

/// The durable job type.
pub const RECEIPT_SEND: JobType<'static> = JobType::new("receipt.send");

/// Attempts a receipt may consume. Only delivery failures and the
/// pre-publication window consume attempts; a halt does not.
pub const MAX_ATTEMPTS: i32 = 25;

/// Time kept inside the native deadline for recording the sent receipt.
const RECORD_RESERVE: Duration = Duration::from_secs(2);

/// How long a receipt whose pin cannot be replayed waits before its next
/// attempt; a compatible build may be deployed meanwhile.
const PIN_RETRY: Duration = Duration::from_secs(300);

/// The payload: only the order's key. Everything the job acts on, the pinned
/// decision included, is read from the order row.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReceiptPayload {
    pub order_ref: String,
}

/// Native payload contract shared by enqueue and the handler.
pub struct ReceiptSend;

impl JobContract for ReceiptSend {
    type Payload = ReceiptPayload;

    fn spec() -> JobSpec {
        JobSpec::new(RECEIPT_SEND).expect("receipt.send is a valid job type")
    }
}

/// Which receipt an order gets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReceiptMode {
    Express,
    Standard,
}

impl ReceiptMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Express => "express",
            Self::Standard => "standard",
        }
    }
}

/// The mode a replayed pin decides. Every arm is answered: an honoured pin is
/// the decision taken at acceptance; anything else defers the receipt rather
/// than reading the flag's current value, which may have changed since.
pub fn mode_from(resolution: PinnedResolution<'_, Toggle>) -> Result<ReceiptMode, JobFailure> {
    match resolution {
        PinnedResolution::Honoured(decision) => Ok(match decision.value() {
            Toggle::On => ReceiptMode::Express,
            Toggle::Off => ReceiptMode::Standard,
        }),
        // `express_receipt` declares no governing switch, so this needs a
        // catalog change to occur; acting on the code default would not be
        // the accepted decision, so wait for the switch instead.
        PinnedResolution::Overridden { .. } => Err(JobFailure::retryable(
            "receipt.pin_overridden",
            "A governing kill switch overrides the pinned receipt decision.",
        )
        .retry_not_before_delay(PIN_RETRY)),
        // Never `fall_back_to_live`: the live value is not the accepted one.
        // Dropping the refusal is the deferral the flags API prescribes.
        PinnedResolution::Incompatible(refusal) => {
            let rejection = refusal.rejection();
            tracing::warn!(
                target: "checkout_service",
                rejection = rejection.error_type(),
                "receipt pin cannot be replayed by this build"
            );
            Err(JobFailure::retryable(
                "receipt.pin_incompatible",
                "The pinned receipt decision cannot be replayed by this build.",
            )
            .retry_not_before_delay(PIN_RETRY))
        }
    }
}

/// The receipt handler. It owns only what it reads: the pool and the handle.
pub struct SendReceipt {
    pool: PgPool,
    express_receipt: ReleaseFlag<Toggle>,
}

/// The gate: a tripped switch continues the job later instead of dropping it.
pub fn gate_policy() -> GatePolicy<DirectWhenTripped> {
    let recheck = Recheck::after(Duration::from_secs(30)).expect("30 s is a valid recheck");
    GatePolicy {
        when_tripped: DirectWhenTripped::Defer(recheck),
        awaiting: recheck,
    }
}

/// The gated handler, validated against the flags before registration.
pub fn gated(
    flags: &Flags,
    handles: &Handles,
    pool: PgPool,
) -> Result<GatedHandler<TypedFlagAware<SendReceipt>>, GateError> {
    let handler = SendReceipt {
        pool,
        express_receipt: handles.express_receipt.clone(),
    };
    flags.gated_direct(
        handler.into_flag_aware(),
        &handles.checkout_halt,
        gate_policy(),
    )
}

/// The native catalog holding the one gated job type.
pub fn catalog(
    flags: &Flags,
    handles: &Handles,
    pool: PgPool,
) -> Result<JobCatalog, batter::BoxError> {
    let handler = gated(flags, handles, pool)?;
    let overrides = JobCatalogDefinitionOverrides::new()
        .max_attempts(MAX_ATTEMPTS)
        .timeout_seconds(30);
    JobCatalog::new()
        .try_handler_with_definition_overrides(handler, overrides)
        .map_err(|error: CatalogError| error.into())
}

fn phases_rejected(rejection: JobPhasesRejection) -> JobFailure {
    match rejection {
        JobPhasesRejection::Exhausted | JobPhasesRejection::Ended => {
            JobFailure::timeout("receipt.no_work_time", "No work time remained.")
        }
        JobPhasesRejection::Unsupported | JobPhasesRejection::Reserve(_) => JobFailure::terminal(
            "receipt.phases_unavailable",
            "Invocation phases are unavailable.",
        ),
    }
}

/// Placeholder for the provider call that delivers the receipt (for example
/// through `batter::outbound_http`). It runs under the work phase only.
async fn deliver(
    context: &OperationContext,
    order: &orders::PendingReceipt,
    mode: ReceiptMode,
) -> Result<(), JobFailure> {
    let (order_ref, recipient) = (order.order_ref.clone(), order.recipient.clone());
    context
        .run("receipt.deliver", |_scope| async move {
            tracing::info!(
                target: "checkout_service",
                order_ref, recipient_len = recipient.len(), mode = mode.as_str(),
                "receipt delivered"
            );
            Ok::<_, std::convert::Infallible>(())
        })
        .await
        .map_err(|error| match error {
            OperationError::Interrupted(_) => JobFailure::timeout(
                "receipt.delivery_unknown",
                "Receipt delivery was interrupted; its outcome is unknown.",
            ),
            OperationError::Failed(never) => match never {},
        })
}

#[async_trait]
impl FlagAwareTypedHandler for SendReceipt {
    type Contract = ReceiptSend;

    async fn execute(
        &self,
        execution: JobExecution<'_>,
        flags: &JobFlags,
        payload: ReceiptPayload,
    ) -> Result<JobCompletion, JobFailure> {
        let phases = job_phases(execution, RECORD_RESERVE).map_err(phases_rejected)?;
        let order =
            match orders::load_receipt(&self.pool, phases.work(), &payload.order_ref).await? {
                orders::ReceiptState::Pending(order) => order,
                // A retry after the record committed but before the job's own
                // completion did: the receipt is already sent.
                orders::ReceiptState::Sent => return Ok(JobCompletion::success()),
            };
        let pin: Pinned<Toggle> = serde_json::from_value(order.express_receipt_pin.clone())
            .map_err(|_| {
                JobFailure::terminal(
                    "receipt.pin_unreadable",
                    "The stored pin is not a pin envelope.",
                )
            })?;
        let mode = mode_from(flags.resolve_pinned(&self.express_receipt, &pin))?;
        deliver(phases.work(), &order, mode).await?;
        // The final-state write keeps its reserve even if work ran long.
        orders::record_receipt(&self.pool, phases.finalization(), &order.order_ref, mode).await?;
        Ok(JobCompletion::success())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use batter::flags::{core::PinRejection, test::TestFlags};

    fn fixture(express: Toggle) -> (TestFlags, Handles) {
        let declared = crate::flags::declare().unwrap();
        let flags = TestFlags::builder(declared.catalog, declared.serving)
            .unwrap()
            .set(&declared.handles.express_receipt, express)
            .release(&declared.handles.checkout_halt)
            .build()
            .unwrap();
        (flags, declared.handles)
    }

    #[test]
    fn the_pin_taken_at_acceptance_decides_after_the_flag_changes() {
        // Accepted while express_receipt was on; the pin is what the order row keeps.
        let (accepting, handles) = fixture(Toggle::On);
        let stored = serde_json::to_value(accepting.view().pin(&handles.express_receipt)).unwrap();

        // The flag is switched off (in a later build of the catalog) before the job runs.
        let (running, handles) = fixture(Toggle::Off);
        assert_eq!(
            running.view().evaluate(&handles.express_receipt).value(),
            Toggle::Off
        );
        let pin: Pinned<Toggle> = serde_json::from_value(stored).unwrap();
        let mode = mode_from(
            running
                .view()
                .resolve_pinned(&handles.express_receipt, &pin),
        );
        assert_eq!(mode.unwrap(), ReceiptMode::Express);
    }

    #[test]
    fn an_unreplayable_pin_defers_instead_of_reading_the_live_value() {
        let (running, handles) = fixture(Toggle::On);
        // A row written by a build whose flag declared another variant.
        let mut stored =
            serde_json::to_value(running.view().pin(&handles.express_receipt)).unwrap();
        stored["variant"] = "turbo".into();
        let pin: Pinned<Toggle> = serde_json::from_value(stored).unwrap();
        let view = running.view();
        let resolution = view.resolve_pinned(&handles.express_receipt, &pin);
        assert_eq!(resolution.rejection(), Some(PinRejection::UnknownVariant));
        let failure = mode_from(resolution).unwrap_err();
        assert_eq!(failure.code, "receipt.pin_incompatible");
    }

    #[tokio::test]
    async fn a_tripped_halt_continues_receipts_instead_of_dropping_them() {
        let policy = gate_policy();
        assert!(matches!(policy.when_tripped, DirectWhenTripped::Defer(_)));
        let (flags, handles) = fixture(Toggle::Off);
        // The gate validates the switch against these flags before registration.
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://checkout@127.0.0.1:1/checkout")
            .unwrap();
        let catalog = catalog(flags.flags(), &handles, pool).unwrap();
        assert!(catalog.contains(RECEIPT_SEND));
    }

    #[test]
    fn the_payload_contract_round_trips() {
        let payload = ReceiptPayload {
            order_ref: "order-1".into(),
        };
        let submission = ReceiptSend::submit(&payload).unwrap();
        assert_eq!(submission.job_type, RECEIPT_SEND);
        assert_eq!(
            serde_json::from_value::<ReceiptPayload>(submission.payload).unwrap(),
            payload
        );
    }
}
