mod error;
pub(crate) mod scope;

pub use error::{CommitUnconfirmed, PgScopeError, PgScopeFailure, PgScopeLoss, PgTransactionError};
pub use scope::PgScopedSql;
use scope::{ScopeSavepoint, command, normalize};

use crate::PgLease;
use sqlx::PgPool;

/// Low-level READ COMMITTED, READ WRITE PostgreSQL transaction owner.
/// Prefer [`crate::run_atomic`], which withholds outputs until completion.
///
/// Every SQL scope consumes the owner. Cancellation, panic, boundary loss and
/// all completion paths retire the physical connection; retirement does not
/// confirm server rollback. Acquisition resets session state before BEGIN. The
/// top-level XID is assigned at birth and checked after every scope and before
/// completion. Use PostgreSQL 18 or later.
///
/// ```no_run
/// use batter_sqlx::low_level::PgAtomicTransaction;
/// # async fn example(pool: &sqlx::PgPool) -> Result<(), Box<dyn std::error::Error>> {
/// let tx = PgAtomicTransaction::begin(pool).await?;
/// let (tx, ()) = tx.application(async |sql| {
///     sqlx::query("INSERT INTO audit_events (message) VALUES ('submitted')")
///         .execute(sql.executor()).await?;
///     Ok::<_, sqlx::Error>(())
/// }).await?;
/// let committed = tx.commit().await?;
/// # let _ = committed;
/// # Ok(()) }
/// ```
/// Cancellation cannot hand the same owner back to its caller:
/// ```compile_fail,E0382
/// async fn cancelled(tx: batter_sqlx::low_level::PgAtomicTransaction) {
///     let future = tx.application(async |_| Ok::<_, sqlx::Error>(()));
///     drop(future);
///     tx.commit().await.unwrap();
/// }
/// ```
/// ```compile_fail,E0599
/// fn extract(mut tx: batter_sqlx::low_level::PgAtomicTransaction) {
///     let _ = tx.executor();
/// }
/// ```
/// ```compile_fail,E0451
/// fn forge() {
///     let _ = batter_sqlx::low_level::PgAtomicTransaction { lease: todo!(), xid: String::new() };
/// }
/// ```
#[must_use = "complete the transaction through a consuming operation"]
pub struct PgAtomicTransaction {
    lease: PgLease,
    xid: String,
}

/// Acknowledgement of this owner's COMMIT, not an authorization to replay.
#[derive(Debug)]
#[must_use]
pub struct PgCommitConfirmed(());

/// Acknowledgement of this owner's ROLLBACK.
#[derive(Debug)]
#[must_use]
pub struct PgRollbackConfirmed(());

impl std::fmt::Debug for PgAtomicTransaction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PgAtomicTransaction")
    }
}

impl PgAtomicTransaction {
    /// Acquire and begin a fresh transaction. Bound the entire consuming workflow
    /// with an OperationContext when a deadline/cancellation lineage is required.
    pub async fn begin(pool: &PgPool) -> Result<Self, PgTransactionError> {
        let mut lease = PgLease {
            connection: Some(pool.acquire().await?),
        };
        normalize(lease.connection_mut()).await?;
        command(
            lease.connection_mut(),
            "BEGIN ISOLATION LEVEL READ COMMITTED READ WRITE",
        )
        .await?;
        let xid = sqlx::query_scalar("SELECT pg_catalog.pg_current_xact_id()::text")
            .fetch_one(lease.connection_mut())
            .await?;
        Ok(Self { lease, xid })
    }

    /// Run application SQL. Any returned application error consumes the owner.
    /// Application savepoints are confined to this scope. Explicit transaction
    /// control is detected after execution; already committed effects cannot be
    /// undone. Values returned from SQL remain provisional until commit.
    pub async fn application<T, E>(
        self,
        work: impl AsyncFnOnce(&mut PgScopedSql<'_>) -> Result<T, E>,
    ) -> Result<(Self, T), PgScopeError<E>> {
        let (owner, result) = self.operation(work).await.map_err(PgScopeError::Terminal)?;
        match result {
            Ok(value) => Ok((owner, value)),
            Err(error) => Err(PgScopeError::Application(error)),
        }
    }

    /// Run a composable library operation inside its own savepoint.
    ///
    /// `Ok((owner, Err(error)))` means savepoint rollback and revalidation both
    /// succeeded, so none of this operation's transactional writes remain.
    /// Nontransactional effects such as sequence increments are not reversed.
    /// Terminal errors preserve the primary error when recovery also fails.
    /// Cancellation never returns an owner, including during savepoint cleanup.
    ///
    /// ```no_run
    /// # async fn example(tx: batter_sqlx::low_level::PgAtomicTransaction) -> Result<(), Box<dyn std::error::Error>> {
    /// let (tx, result) = tx.operation(async |sql| {
    ///     sqlx::query("INSERT INTO audit_events (message) VALUES ('operation')")
    ///         .execute(sql.executor()).await?;
    ///     Ok::<_, sqlx::Error>(())
    /// }).await?;
    /// match result {
    ///     Ok(()) => { let _ = tx.commit().await?; }
    ///     Err(_) => { let _ = tx.rollback().await?; }
    /// }
    /// # Ok(()) }
    /// ```
    pub async fn operation<T, E>(
        mut self,
        work: impl AsyncFnOnce(&mut PgScopedSql<'_>) -> Result<T, E>,
    ) -> Result<(Self, Result<T, E>), PgScopeFailure<E>> {
        self.validate().await?;
        let savepoint = ScopeSavepoint::begin(self.lease.connection_mut()).await?;
        let result = work(&mut PgScopedSql {
            connection: self.lease.connection_mut(),
        })
        .await;
        match result {
            Ok(value) => {
                self.validate().await?;
                savepoint.release(self.lease.connection_mut()).await?;
                self.validate().await?;
                Ok((self, Ok(value)))
            }
            Err(application) => {
                // An aborted subtransaction cannot execute the validation SELECT
                // until rollback. A missing guard cannot yield a reusable owner.
                let recovery = async {
                    savepoint.rollback(self.lease.connection_mut()).await?;
                    self.validate().await
                }
                .await;
                match recovery {
                    Ok(()) => Ok((self, Err(application))),
                    Err(recovery) => Err(PgScopeFailure::Recovery {
                        application,
                        recovery: recovery.into(),
                    }),
                }
            }
        }
    }

    /// Validate continuity, then commit with no intervening application code.
    /// There is no await after the acknowledged COMMIT before returning evidence.
    /// Dropping this future may lose acknowledgement even if PostgreSQL commits.
    pub async fn commit(mut self) -> Result<PgCommitConfirmed, CommitUnconfirmed> {
        self.validate().await.map_err(CommitUnconfirmed)?;
        command(self.lease.connection_mut(), "COMMIT")
            .await
            .map_err(CommitUnconfirmed)?;
        // The lease retires even after acknowledgement: arbitrary SQL may have
        // changed session settings or acquired session advisory locks.
        Ok(PgCommitConfirmed(()))
    }

    /// Validate continuity and acknowledge rollback of this exact transaction.
    pub async fn rollback(mut self) -> Result<PgRollbackConfirmed, PgTransactionError> {
        self.validate().await?;
        command(self.lease.connection_mut(), "ROLLBACK").await?;
        Ok(PgRollbackConfirmed(()))
    }

    async fn validate(&mut self) -> Result<(), PgTransactionError> {
        let (xid, isolation, read_only): (Option<String>, String, String) = sqlx::query_as(
            "SELECT pg_catalog.pg_current_xact_id_if_assigned()::text, \
             pg_catalog.current_setting('transaction_isolation'), \
             pg_catalog.current_setting('transaction_read_only')",
        )
        .fetch_one(self.lease.connection_mut())
        .await?;
        if xid.as_deref() != Some(self.xid.as_str()) {
            return Err(PgTransactionError::TransactionBoundaryLost);
        }
        if isolation != "read committed" {
            return Err(PgTransactionError::TransactionIsolationChanged);
        }
        if read_only != "off" {
            return Err(PgTransactionError::TransactionAccessChanged);
        }
        Ok(())
    }
}
