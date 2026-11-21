//! The application's own `checkout_orders` table.

use std::{error::Error, fmt};

use batter::{
    operation::{OperationContext, OperationError},
    runledger::{
        PgAtomicError, PgScopeError, RunledgerDatabase,
        native::{
            core::jobs::{JobContract, JobFailure},
            postgres::{self, jobs::JobEnqueue},
        },
        run_atomic,
    },
};
use sqlx::PgPool;

use crate::receipts::{ReceiptMode, ReceiptPayload, ReceiptSend};

/// A validated checkout request.
#[derive(Clone, Debug)]
pub struct NewOrder {
    pub order_ref: String,
    pub recipient: String,
}

/// What accepting an order did. A repeated `order_ref` is the same checkout:
/// it keeps the pin and the receipt job of its first acceptance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Acceptance {
    Accepted,
    AlreadyAccepted,
}

/// Why the acceptance transaction rejected its own body. Its causes are
/// retained for trusted inspection; formatting names only the step.
pub enum AcceptFailure {
    Insert(PgScopeError<sqlx::Error>),
    Enqueue(PgScopeError<postgres::Error>),
}

impl fmt::Display for AcceptFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Insert(_) => "the order insert failed",
            Self::Enqueue(_) => "the receipt enqueue failed",
        })
    }
}

impl fmt::Debug for AcceptFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl Error for AcceptFailure {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Insert(cause) => Some(cause),
            Self::Enqueue(cause) => Some(cause),
        }
    }
}

/// Inserts the order with its pinned decision and enqueues its receipt in one
/// transaction. Outputs leave only after acknowledged commit; an unconfirmed
/// commit is `PgAtomicError::Uncertain`, and a retry with the same `order_ref`
/// is then answered from the row, never inserted twice.
#[allow(
    clippy::result_large_err,
    reason = "the runner's exhaustive outcome retains its causes unboxed"
)]
pub async fn accept(
    database: &RunledgerDatabase,
    order: &NewOrder,
    express_receipt_pin: &serde_json::Value,
) -> Result<Acceptance, PgAtomicError<Acceptance, AcceptFailure>> {
    let payload = ReceiptPayload {
        order_ref: order.order_ref.clone(),
    };
    let submission = ReceiptSend::submit(&payload)
        .expect("the receipt contract is enabled and its payload serializes")
        .idempotency_key(format!("receipt.send:{}", order.order_ref));
    let request = JobEnqueue::from(&submission);
    run_atomic(database, async |mut scope| {
        let inserted = scope
            .application(async |sql| {
                sqlx::query_scalar::<_, String>(
                    "INSERT INTO checkout_orders (order_ref, recipient, express_receipt_pin) \
                     VALUES ($1, $2, $3) ON CONFLICT (order_ref) DO NOTHING RETURNING order_ref",
                )
                .bind(&order.order_ref)
                .bind(&order.recipient)
                .bind(express_receipt_pin)
                .fetch_optional(sql.executor())
                .await
            })
            .await
            .map_err(AcceptFailure::Insert)?;
        if inserted.is_none() {
            return Ok(Acceptance::AlreadyAccepted);
        }
        scope
            .queue()
            .enqueue_job(&request)
            .await
            .map_err(AcceptFailure::Enqueue)?;
        Ok(Acceptance::Accepted)
    })
    .await
}

/// An order whose receipt has not been recorded yet.
#[derive(Debug)]
pub struct PendingReceipt {
    pub order_ref: String,
    pub recipient: String,
    pub express_receipt_pin: serde_json::Value,
}

/// The receipt state of one order, read in one statement.
#[derive(Debug)]
pub enum ReceiptState {
    Pending(PendingReceipt),
    Sent,
}

type ReceiptRow = (String, serde_json::Value, Option<String>);

/// Reads the order a receipt job names, under the job's work phase.
pub async fn load_receipt(
    pool: &PgPool,
    context: &OperationContext,
    order_ref: &str,
) -> Result<ReceiptState, JobFailure> {
    let row = context
        .run("receipt.load_order", |_scope| async {
            sqlx::query_as::<_, ReceiptRow>(
                "SELECT recipient, express_receipt_pin, receipt_mode \
                 FROM checkout_orders WHERE order_ref = $1",
            )
            .bind(order_ref)
            .fetch_optional(pool)
            .await
        })
        .await
        .map_err(|error| match error {
            OperationError::Failed(_) => {
                JobFailure::retryable("receipt.order_unreadable", "The order could not be read.")
            }
            OperationError::Interrupted(_) => JobFailure::timeout(
                "receipt.order_read_interrupted",
                "Reading the order was interrupted.",
            ),
        })?;
    match row {
        // Orders and their receipts commit together, so this is a payload that
        // names no order: retrying cannot help.
        None => Err(JobFailure::terminal(
            "receipt.order_missing",
            "The receipt names no order.",
        )),
        Some((_, _, Some(_))) => Ok(ReceiptState::Sent),
        Some((recipient, express_receipt_pin, None)) => Ok(ReceiptState::Pending(PendingReceipt {
            order_ref: order_ref.to_owned(),
            recipient,
            express_receipt_pin,
        })),
    }
}

/// Records the sent receipt once; a second attempt changes nothing.
pub async fn record_receipt(
    pool: &PgPool,
    context: &OperationContext,
    order_ref: &str,
    mode: ReceiptMode,
) -> Result<(), JobFailure> {
    context
        .run("receipt.record", |_scope| async {
            sqlx::query(
                "UPDATE checkout_orders SET receipt_mode = $2, receipt_sent_at = now() \
                 WHERE order_ref = $1 AND receipt_mode IS NULL",
            )
            .bind(order_ref)
            .bind(mode.as_str())
            .execute(pool)
            .await
        })
        .await
        .map(|_| ())
        .map_err(|error| match error {
            OperationError::Failed(_) => {
                JobFailure::retryable("receipt.unrecorded", "The sent receipt was not recorded.")
            }
            OperationError::Interrupted(_) => JobFailure::timeout(
                "receipt.record_unknown",
                "Recording the receipt was interrupted; whether it committed is unknown.",
            ),
        })
}
