use sqlx::{Executor, PgConnection, Postgres};

/// Opaque SQL execution, not evidence of transaction identity or disposition.
/// Native query executors cannot expose or replace the retained connection.
pub trait PgExecutor: Send {
    /// Borrow this capability for one native SQLx operation.
    fn executor(&mut self) -> impl Executor<'_, Database = Postgres>;
}

/// Low-level borrow-scoped SQL on the exact connection owned by a [`crate::PgLease`].
/// This is not a transaction API. Use [`crate::run_atomic`] for atomic work or
/// [`crate::PgReadOnlySnapshot`] for coherent inspection. Raw transaction-control
/// SQL is an escape hatch without typed completion or result-retention guarantees.
///
/// ```compile_fail,E0614
/// fn cannot_extract_connection(session: &mut batter_sqlx::PgSession<'_>) -> &mut sqlx::PgConnection {
///     &mut **session
/// }
/// ```
/// No second manual transaction protocol is available from a session:
/// ```compile_fail,E0599
/// async fn cannot_begin(session: &mut batter_sqlx::PgSession<'_>) {
///     session.begin().await;
/// }
/// ```
/// ```compile_fail,E0432
/// use batter_sqlx::PgTransaction;
/// ```
#[must_use = "the session is the lease's only SQL execution capability"]
pub struct PgSession<'connection> {
    connection: &'connection mut PgConnection,
}

/// Private evidence that the exact acquired connection is idle for pool return.
pub(crate) struct PoolReturnReady(());

impl<'connection> PgSession<'connection> {
    pub(crate) fn new(connection: &'connection mut PgConnection) -> Self {
        Self { connection }
    }

    /// Execute low-level SQL; this does not acknowledge any transaction outcome.
    pub fn executor(&mut self) -> impl Executor<'_, Database = Postgres> {
        &mut *self.connection
    }

    pub(crate) fn native_connection(&mut self) -> &mut PgConnection {
        self.connection
    }

    /// Synchronize raw SQL to an idle transaction boundary, not a full session reset.
    pub(crate) async fn prepare_pool_return(&mut self) -> Option<PoolReturnReady> {
        // BEGIN can fail for an aborted transaction. Only the final acknowledged
        // ROLLBACK authorizes pool return; failed/cancelled cleanup retires instead.
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
