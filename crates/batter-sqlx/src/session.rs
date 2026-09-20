use sqlx::{Connection, Executor, PgConnection, Postgres, Transaction};

/// Opaque PostgreSQL execution capability.
///
/// Implementations may represent a checked-out session or an explicit
/// transaction. The returned SQLx executor is borrow-scoped: callers can run
/// native queries but cannot name, replace, detach, or retain the underlying
/// connection.
pub trait PgExecutor: Send {
    /// Borrow this capability for one native SQLx operation.
    fn executor(&mut self) -> impl Executor<'_, Database = Postgres>;
}

/// Borrow-scoped access to the exact connection owned by a [`crate::PgLease`].
///
/// This type deliberately implements neither `DerefMut` nor `AsMut` for
/// [`PgConnection`]. Native SQL remains available through [`PgExecutor`], and
/// explicit transactions begin through [`Self::begin`]. The physical
/// connection therefore cannot be replaced while the lease later decides
/// whether that same connection returns to its pool.
///
/// ```compile_fail,E0614
/// fn cannot_extract_connection(
///     session: &mut batter_sqlx::PgSession<'_>,
/// ) -> &mut sqlx::PgConnection {
///     &mut **session
/// }
/// ```
#[must_use = "the session is the lease's only SQL execution capability"]
pub struct PgSession<'connection> {
    connection: &'connection mut PgConnection,
    unacknowledged_transactions: usize,
}

/// Private evidence that the acquired physical connection is eligible for its
/// one pool-return transition.
pub(crate) struct PoolReturnReady(());

impl<'connection> PgSession<'connection> {
    pub(crate) fn new(connection: &'connection mut PgConnection) -> Self {
        Self {
            connection,
            unacknowledged_transactions: 0,
        }
    }

    /// Borrow this exact session for one native SQLx operation.
    pub fn executor(&mut self) -> impl Executor<'_, Database = Postgres> {
        &mut *self.connection
    }

    /// Begin an explicit transaction that cannot expose or replace its session.
    ///
    /// A successful [`PgTransaction::commit`] or [`PgTransaction::rollback`]
    /// is required before the enclosing lease may return this connection to
    /// its pool. Dropping or forgetting the returned value keeps the session
    /// ineligible for pool return.
    pub async fn begin(&mut self) -> Result<PgTransaction<'_>, sqlx::Error> {
        self.unacknowledged_transactions = self.unacknowledged_transactions.saturating_add(1);
        let transaction = self.connection.begin().await?;
        Ok(PgTransaction {
            transaction,
            unacknowledged_transactions: &mut self.unacknowledged_transactions,
        })
    }

    pub(crate) fn native_connection(&mut self) -> &mut PgConnection {
        self.connection
    }

    /// Establish an idle PostgreSQL transaction state before pool return.
    ///
    /// An opaque native executor necessarily accepts transaction-control SQL,
    /// so the typed child counter alone cannot prove that the server is idle.
    /// The boundary first attempts `BEGIN`, then always attempts `ROLLBACK`.
    /// An idle session completes both without PostgreSQL's outside-transaction
    /// warning. An already-open transaction can warn on `BEGIN`, and a failed
    /// transaction rejects it; neither result authorizes return. Only the final
    /// successful `ROLLBACK`, which synchronizes every case to PostgreSQL's idle
    /// ReadyForQuery state, constructs the witness. Failure or cancellation
    /// yields no witness; the owning lease then retires the connection.
    pub(crate) async fn prepare_pool_return(&mut self) -> Option<PoolReturnReady> {
        if self.unacknowledged_transactions != 0 {
            return None;
        }
        // Ignore this result deliberately. Whether BEGIN starts an empty
        // transaction, warns because one is already open, or errors because the
        // transaction is failed, the mandatory ROLLBACK below is the sole proof.
        let _ = sqlx::raw_sql("BEGIN").execute(&mut *self.connection).await;
        sqlx::raw_sql("ROLLBACK")
            .execute(&mut *self.connection)
            .await
            .ok()?;
        Some(PoolReturnReady(()))
    }
}

impl PgExecutor for PgSession<'_> {
    fn executor(&mut self) -> impl Executor<'_, Database = Postgres> {
        self.executor()
    }
}

impl std::fmt::Debug for PgSession<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PgSession")
    }
}

/// Opaque explicit transaction begun from a [`PgSession`].
///
/// SQL executes through [`PgExecutor`]. Only consuming [`Self::commit`] and
/// [`Self::rollback`] can acknowledge completion to the session; the
/// underlying SQLx transaction and connection are never returned. Dropping or
/// forgetting this value leaves the enclosing lease ineligible for pool return.
///
/// ```compile_fail,E0614
/// fn cannot_extract_transaction(
///     transaction: &mut batter_sqlx::PgTransaction<'_>,
/// ) -> &mut sqlx::Transaction<'_, sqlx::Postgres> {
///     &mut **transaction
/// }
/// ```
#[must_use = "explicitly commit or roll back the transaction before completing the lease work"]
pub struct PgTransaction<'connection> {
    transaction: Transaction<'connection, Postgres>,
    unacknowledged_transactions: &'connection mut usize,
}

impl<'connection> PgTransaction<'connection> {
    /// Borrow this exact transaction for one native SQLx operation.
    pub fn executor(&mut self) -> impl Executor<'_, Database = Postgres> {
        &mut *self.transaction
    }

    /// Commit and consume this transaction.
    pub async fn commit(self) -> Result<(), sqlx::Error> {
        let Self {
            transaction,
            unacknowledged_transactions,
        } = self;
        let outcome = transaction.commit().await;
        if outcome.is_ok() {
            *unacknowledged_transactions = unacknowledged_transactions.saturating_sub(1);
        }
        outcome
    }

    /// Roll back and consume this transaction.
    pub async fn rollback(self) -> Result<(), sqlx::Error> {
        let Self {
            transaction,
            unacknowledged_transactions,
        } = self;
        let outcome = transaction.rollback().await;
        if outcome.is_ok() {
            *unacknowledged_transactions = unacknowledged_transactions.saturating_sub(1);
        }
        outcome
    }
}

impl PgExecutor for PgTransaction<'_> {
    fn executor(&mut self) -> impl Executor<'_, Database = Postgres> {
        self.executor()
    }
}

impl std::fmt::Debug for PgTransaction<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PgTransaction")
    }
}
