//! Explicit integration seam for a transaction owner such as Batter.
//! Calling this with an autocommit executor breaks application atomicity.
//! The owner must reject stale completion, rollback on every error/cancellation,
//! and publish application output only after acknowledged commit. Claim before
//! application writes, then finish with their final authentication outcome in
//! the same transaction. No await follows
//! query decoding; transaction disposition belongs exclusively to the caller.

use super::{
    PgAttemptClaim, PgAttemptClaimResult, PgAttemptReceipt, completion_query, completion_result,
};
use runlimit_core::attempts::{AttemptOutcome, StagedAttemptCompletion};
use sqlx::{Executor, Postgres, Row};
/// Locks and fences a live reservation before application writes. The lease
/// is validated after acquiring the lock, against authoritative server time.
/// Hold the same transaction through [`finish_in`] and final commit. An
/// autocommit executor produces a claim unusable in subsequent transactions.
/// Rolling back the savepoint containing this operation invalidates the
/// claim by reverting its private token rotation; releasing that savepoint
/// retains the lock and the claim remains valid in the outer transaction.
///
/// # Errors
/// Returns database/decode failures; the transaction owner must rollback.
pub async fn claim_in<'e, E: Executor<'e, Database = Postgres>>(
    executor: E,
    mut receipt: PgAttemptReceipt,
) -> Result<PgAttemptClaimResult, sqlx::Error> {
    let row = super::sql::CLAIM_SQL
        .query()
        .bind(receipt.policy.fingerprint().into_bytes().to_vec())
        .bind(receipt.subject.to_vec())
        .bind(&receipt.token)
        .fetch_optional(executor)
        .await?;
    let decode = |error| {
        sqlx::Error::Decode(Box::new(crate::CheckError::storage_decode_invariant(
            "decode attempt claim response",
            error,
        )))
    };
    match row {
        Some(row) => {
            receipt.token = row.try_get("lease_token").map_err(decode)?;
            Ok(PgAttemptClaimResult::Claimed(PgAttemptClaim {
                receipt,
                transaction_id: row.try_get("transaction_id").map_err(decode)?,
            }))
        }
        None => Ok(PgAttemptClaimResult::Stale),
    }
}
/// Stages the final outcome after application checks, in the claim's exact
/// transaction. Time elapsed since claim does not invalidate its held lock.
/// A different transaction returns `Stale` without changing state. The owner
/// must rollback on `Stale` and publish only after acknowledged commit.
///
/// # Errors
/// Returns database/decode failures; the transaction owner must rollback.
pub async fn finish_in<'e, E: Executor<'e, Database = Postgres>>(
    executor: E,
    claim: PgAttemptClaim,
    outcome: AttemptOutcome,
) -> Result<StagedAttemptCompletion, sqlx::Error> {
    let rows = completion_query(&claim.receipt, outcome, Some(&claim.transaction_id))
        .fetch_all(executor)
        .await?;
    completion_result(&rows, outcome).map_err(|error| sqlx::Error::Decode(Box::new(error)))
}
/// Stages one fenced completion in a caller-owned transaction, in one SQL statement.
///
/// # Errors
/// Returns database/decode errors. The transaction owner must rollback;
/// an error does not authorize retrying or publishing application output.
pub async fn complete_in<'e, E: Executor<'e, Database = Postgres>>(
    executor: E,
    receipt: PgAttemptReceipt,
    outcome: AttemptOutcome,
) -> Result<StagedAttemptCompletion, sqlx::Error> {
    let rows = completion_query(&receipt, outcome, None)
        .fetch_all(executor)
        .await?;
    completion_result(&rows, outcome).map_err(|error| sqlx::Error::Decode(Box::new(error)))
}
