use super::{CommitUnconfirmed, PgAtomicTransaction, PgCommitConfirmed, PgTransactionError};
use crate::deadline::until;
use std::{error::Error, fmt};
use tokio::time::Instant;

/// Why a deadline-bound commit did not return an acknowledgement of success.
///
/// `Interrupted`, `DeadlineBeforeCommit` and `Validation` mean COMMIT was never
/// sent: nothing committed and the connection is retired. Every returned owner
/// already passed continuity validation after its last SQL, so a failed check
/// here cannot hide transaction control that committed earlier work.
/// `AcknowledgementDeadline` and `Unconfirmed` leave the outcome unknown; a
/// missing acknowledgement never authorizes replay.
pub enum PgCommitError {
    /// The interrupt completed before submission.
    Interrupted,
    /// The deadline elapsed before submission.
    DeadlineBeforeCommit,
    /// The continuity check failed before submission.
    Validation(PgTransactionError),
    /// COMMIT was submitted and its acknowledgement deadline elapsed.
    AcknowledgementDeadline,
    /// The submitted COMMIT returned a native failure.
    Unconfirmed(CommitUnconfirmed),
}

impl fmt::Display for PgCommitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Interrupted => "interrupted before COMMIT",
            Self::DeadlineBeforeCommit => "deadline elapsed before COMMIT",
            Self::Validation(_) => "transaction validation failed before COMMIT",
            Self::AcknowledgementDeadline => "COMMIT acknowledgement deadline elapsed",
            Self::Unconfirmed(_) => "COMMIT was not confirmed",
        })
    }
}
impl fmt::Debug for PgCommitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
impl Error for PgCommitError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Validation(e) => Some(e),
            Self::Unconfirmed(e) => Some(e),
            _ => None,
        }
    }
}

impl PgAtomicTransaction {
    /// Validate continuity under the absolute deadline, then check the clock
    /// again immediately before polling COMMIT. Interrupts stop validation.
    /// Once submission starts, wait for the native result or the deadline.
    /// Dropping this future during submission can lose the acknowledgement.
    ///
    /// Migration: `commit_unless(interrupt)` is replaced by
    /// `commit_until(deadline, interrupt)`. An outer timeout around the former
    /// could not tell expiry before COMMIT from a lost acknowledgement.
    ///
    /// ```no_run
    /// # async fn example(tx: batter_sqlx::low_level::PgAtomicTransaction, deadline: tokio::time::Instant) -> Result<(), batter_sqlx::PgCommitError> {
    /// let _confirmed = tx.commit_until(deadline, std::future::pending()).await?;
    /// # Ok(()) }
    /// ```
    pub async fn commit_until(
        mut self,
        deadline: Instant,
        interrupt: impl Future<Output = ()>,
    ) -> Result<PgCommitConfirmed, PgCommitError> {
        tokio::select! {
            biased;
            () = interrupt => return Err(PgCommitError::Interrupted),
            checked = until(deadline, self.validate()) => checked
                .map_err(|_| PgCommitError::DeadlineBeforeCommit)?
                .map_err(PgCommitError::Validation)?,
        }
        submit(deadline, self.send_commit()).await
    }
}

// Only the first poll hands COMMIT to SQLx. Cancellation of a pending native
// send can no longer establish whether the server committed.
async fn submit<T>(
    deadline: Instant,
    command: impl Future<Output = Result<T, CommitUnconfirmed>>,
) -> Result<T, PgCommitError> {
    if Instant::now() >= deadline {
        return Err(PgCommitError::DeadlineBeforeCommit);
    }
    // timeout_at polls the command first. From that poll onward a missing
    // acknowledgement can no longer establish that nothing committed.
    tokio::time::timeout_at(deadline, command)
        .await
        .map_err(|_| PgCommitError::AcknowledgementDeadline)?
        .map_err(PgCommitError::Unconfirmed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn submission_separates_elapsed_validation_from_unacknowledged_commit() {
        let result = submit::<()>(Instant::now(), async { panic!("late COMMIT") }).await;
        assert!(
            matches!(result, Err(PgCommitError::DeadlineBeforeCommit)),
            "elapsed validation must prevent submission"
        );
        let deadline = Instant::now() + std::time::Duration::from_secs(1);
        let result = submit::<()>(deadline, std::future::pending()).await;
        assert!(
            matches!(result, Err(PgCommitError::AcknowledgementDeadline)),
            "a pending COMMIT has an uncertain outcome"
        );
    }
}
