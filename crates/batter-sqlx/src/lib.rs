//! Library-owned atomic PostgreSQL workflows and read-only snapshots.
//!
//! [`run_atomic`] releases output only after acknowledged commit, with known
//! rejection separate from [`PgAtomicUncertainty`]. [`PgReadOnlySnapshot`] owns
//! coherent read-only inspection. Both retire their session on every path.
//!
//! ```no_run
//! # async fn example(pool: &sqlx::PgPool) -> Result<i64, Box<dyn std::error::Error>> {
//! let id = batter_sqlx::run_atomic(pool, async |scope| {
//!     scope.application(async |sql| {
//!         sqlx::query_scalar::<_, i64>("INSERT INTO records DEFAULT VALUES RETURNING id")
//!             .fetch_one(sql.executor()).await
//!     }).await
//! }).await?;
//! # Ok(id) }
//! ```
//!
//! Exceptional manual ownership is confined to [`low_level`]. The lease/session
//! APIs below are low-level connection disposition, not atomic result evidence.
//!
//! [`PgLease`] retires on drop. [`PgLease::with_connection`] is the only path
//! back to the SQLx pool: after supplied work completes with `Ok`, the boundary
//! synchronizes raw PostgreSQL transaction state with an idle-state handshake
//! ending in `ROLLBACK` and returns only on successful cleanup. It retires when
//! work returns `Err`, cleanup fails, or its future is dropped by a deadline,
//! cancellation or unwinding.
//! Work that must never return its connection uses
//! [`PgLease::with_retiring_connection`], which also
//! consumes the lease. Both methods expose only an opaque [`PgSession`], so safe
//! callers cannot replace the physical connection before disposition. A lease
//! cannot be returned after an interrupted query because no public API can
//! borrow it and later choose a different disposition.
//! Retirement releases local pool capacity, **not** server locks or a confirmed
//! rollback. Detached server sessions can outlive [`PgPool::close`] and exceed
//! the pool's `max_connections`. Applications own remote outcome reconciliation.
//!
#![forbid(unsafe_code)]

mod atomic;
mod atomic_context;
mod atomic_runner;
mod failure;
mod profile;
mod profiled_pool;
mod session;
mod snapshot;

pub mod verification;

pub use atomic::{
    CommitUnconfirmed, PgCommitConfirmed, PgRollbackConfirmed, PgScopeError, PgScopeFailure,
    PgScopeLoss, PgScopedSql, PgTransactionError,
};
pub use atomic_context::{run_atomic_in, run_atomic_profiled_in};
pub use atomic_runner::{
    PgAtomicError, PgAtomicScope, PgAtomicUncertainty, run_atomic, run_atomic_profiled,
};
/// Exceptional consuming composition. Outputs are provisional and completion
/// must be paired by the caller. Prefer [`run_atomic`] on the canonical path.
pub mod low_level {
    pub use crate::atomic::PgAtomicTransaction;
}
pub use failure::{FailureClass, SqlxFailure};
pub use profile::{PgProfileError, PgSessionProfile};
pub use profiled_pool::PgProfiledPool;
use session::PoolReturnReady;
pub use session::{PgExecutor, PgSession};
pub use snapshot::{PgReadOnlySnapshot, PgReadOnlySql, PgSnapshotError};

use batter_core::{
    RegistrationError,
    cleanup::CleanupSlot,
    lifecycle::Supervisor,
    operation::{OperationContext, OperationError},
};
use sqlx::{
    PgConnection, PgPool, Postgres,
    pool::PoolConnection,
    postgres::{PgConnectOptions, PgPoolOptions},
};

/// One checked-out connection, retired unless [`Self::with_connection`] proves pool return ready.
///
/// Keep this owner **inside** the operation future so timeout, caller drop and
/// unwinding destroy it. A lease kept outside a cancelled future remains owned
/// until its caller drops it. Disposition follows the outcome of the work run
/// through [`Self::with_connection`]. [`Self::with_retiring_connection`]
/// consumes the same owner without any pool-return transition. There is no
/// asynchronous Drop, transaction manager, automatic replay, or protection
/// from process death.
///
/// Pool return cannot be requested directly:
///
/// ```compile_fail,E0624
/// fn cannot_return_directly(lease: batter_sqlx::PgLease) {
///     lease.return_to_pool();
/// }
/// ```
#[must_use = "keep the lease inside the bounded operation and choose a consuming disposition method"]
pub struct PgLease {
    connection: Option<PoolConnection<Postgres>>,
}

impl PgLease {
    /// Acquire under the existing total deadline and cancellation lineage.
    ///
    /// Never-polled or already-interrupted calls do not acquire. SQLx owns
    /// cancellation while acquisition is pending; this lease begins only when
    /// acquisition yields a connection. SQLx may discard a connection during
    /// acquisition; no server-session bound is implied at that boundary either.
    pub async fn acquire(
        pool: &PgPool,
        context: &OperationContext,
    ) -> Result<Self, OperationError<SqlxFailure>> {
        context
            .run("postgres.acquire", |_| async {
                let connection = pool.acquire().await.map_err(SqlxFailure::from)?;
                Ok(Self {
                    connection: Some(connection),
                })
            })
            .await
    }

    fn connection_mut(&mut self) -> &mut PgConnection {
        self.connection.as_mut().expect("live lease")
    }

    /// Run work on a connection that is retired for every outcome.
    ///
    /// This consumes the lease and exposes an opaque SQL execution session only
    /// within the supplied future. Completion, error, cancellation, deadline and
    /// unwinding all retire it, so later pool return is unrepresentable. Use
    /// this for deliberately interrupted native operations and for protocols
    /// whose failure leaves remote session state uncertain.
    ///
    /// ```no_run
    /// use batter_core::operation::OperationContext;
    /// use batter_sqlx::{PgLease, SqlxFailure};
    /// use sqlx::PgPool;
    /// # async fn example(pool: &PgPool, ctx: &OperationContext) -> Result<i32, Box<dyn std::error::Error>> {
    /// let lease = PgLease::acquire(pool, ctx).await?;
    /// let backend = lease.with_retiring_connection(async |session| {
    ///     sqlx::query_scalar::<_, i32>("SELECT pg_backend_pid()")
    ///         .fetch_one(session.executor())
    ///         .await
    ///         .map_err(SqlxFailure::from)
    /// }).await?;
    /// # Ok(backend) }
    /// ```
    ///
    /// The old borrow-then-decide transition is intentionally absent:
    ///
    /// ```compile_fail,E0599
    /// fn cannot_borrow_for_later_disposition(mut lease: batter_sqlx::PgLease) {
    ///     let _ = lease.connection();
    /// }
    /// ```
    pub async fn with_retiring_connection<T>(
        mut self,
        work: impl AsyncFnOnce(&mut PgSession<'_>) -> T,
    ) -> T {
        let mut session = PgSession::new(self.connection_mut());
        work(&mut session).await
    }

    /// Run one unit of work on this connection and let its outcome decide disposition.
    ///
    /// `Ok` attempts to synchronize the exact physical connection to an idle
    /// PostgreSQL transaction state before returning it to the SQLx pool;
    /// `Err` retires it. Dropping the returned future before completion,
    /// including during that synchronization or at a deadline or
    /// cancellation boundary, also retires it, so an interrupted query cannot
    /// be followed by pool return. Await it inside the bounding operation so
    /// that boundary owns the drop. SQLx asynchronously checks a returned
    /// connection before reuse and may discard it. This is low-level session
    /// work, not a transaction workflow: use [`run_atomic`] to withhold outputs
    /// until acknowledged disposition. Because the executor can run raw `BEGIN`,
    /// successful work also runs a `BEGIN`/`ROLLBACK` normalization handshake
    /// before pool return. Only the final successful rollback authorizes return;
    /// a cleanup statement that returns failure preserves the application `Ok`
    /// but retires its connection. Dropping this future during cleanup cannot
    /// return the application value; it retires the connection. Applications
    /// whose disposition becomes durable before cleanup must retain that known
    /// outcome outside the cancellable future. The handshake proves an idle
    /// transaction boundary, not a reset of arbitrary session settings,
    /// advisory locks, or prepared transactions.
    ///
    /// ```no_run
    /// use batter_core::operation::OperationContext;
    /// use batter_sqlx::{PgLease, SqlxFailure};
    /// use sqlx::PgPool;
    /// # async fn example(pool: &PgPool, ctx: &OperationContext) -> Result<i64, Box<dyn std::error::Error>> {
    /// let lease = PgLease::acquire(pool, ctx).await?;
    /// let count = ctx.run("accounts.count", |_| lease.with_connection(async |session| {
    ///     sqlx::query_scalar::<_, i64>("SELECT count(*) FROM accounts")
    ///         .fetch_one(session.executor())
    ///         .await
    ///         .map_err(SqlxFailure::from)
    /// })).await?;
    /// # Ok(count) }
    /// ```
    pub async fn with_connection<T, E>(
        mut self,
        work: impl AsyncFnOnce(&mut PgSession<'_>) -> Result<T, E>,
    ) -> Result<T, E> {
        let (outcome, pool_return) = {
            let mut session = PgSession::new(self.connection_mut());
            let outcome = work(&mut session).await;
            let pool_return = if outcome.is_ok() {
                session.prepare_pool_return().await
            } else {
                None
            };
            (outcome, pool_return)
        };
        match outcome {
            Ok(value) => {
                if let Some(ready) = pool_return {
                    self.return_to_pool(ready);
                }
                // Otherwise dropping the lease retires a session with
                // failed raw-state cleanup while
                // preserving the application's successful value.
                Ok(value)
            }
            // Dropping the lease retires the connection.
            Err(error) => Err(error),
        }
    }

    /// Apply an application-selected native SQLx migration bundle on this lease.
    ///
    /// The lease is consumed and the physical connection remains private. Native
    /// SQLx owns migration selection, locking, history and checksum validation;
    /// this adapter owns only connection disposition. Failure or cancellation
    /// retires the connection, including migration errors that leave a native
    /// session advisory lock held. Success follows the normal idle-state proof
    /// before pool return. Await inside the caller's operation budget.
    ///
    /// The original `MigrateError` is retained as `sqlx::Error::Migrate` inside
    /// the redacted [`SqlxFailure`]. No automatic replay or provisioning policy
    /// is introduced.
    pub async fn migrate(self, migrator: &sqlx::migrate::Migrator) -> Result<(), SqlxFailure> {
        self.with_connection(async |session| {
            migrator
                .run_direct(None, session.native_connection(), false)
                .await
                .map_err(sqlx::Error::from)
                .map_err(SqlxFailure::from)
        })
        .await
    }

    /// Native SQLx pool return after acknowledged successful completion.
    fn return_to_pool(mut self, _ready: PoolReturnReady) {
        drop(self.connection.take());
    }
}

impl Drop for PgLease {
    fn drop(&mut self) {
        if let Some(connection) = self.connection.take() {
            drop(connection.detach());
        }
    }
}

impl std::fmt::Debug for PgLease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PgLease")
    }
}

/// Run SELECT 1 with acquisition and query sharing the supplied total budget.
///
/// Success permits native pool reuse. Every unsuccessful acquired lease retires.
/// This performs no creation, migration, schema check, or readiness approval.
/// See the runnable `batter-example-postgres-lifecycle` package for composition.
pub async fn probe(
    pool: &PgPool,
    context: &OperationContext,
) -> Result<(), OperationError<SqlxFailure>> {
    context
        .run("postgres.probe", |_| async {
            // The enclosing boundary already bounds acquisition and query together.
            let connection = pool.acquire().await.map_err(SqlxFailure::from)?;
            let lease = PgLease {
                connection: Some(connection),
            };
            lease
                .with_connection(async |session| {
                    sqlx::query("SELECT 1")
                        .execute(session.executor())
                        .await
                        .map(|_| ())
                        .map_err(SqlxFailure::from)
                })
                .await
        })
        .await
}

/// Construct a native lazy PostgreSQL pool and publish its close finalizer.
///
/// `slot` must be reserved before this call. Successful native construction is
/// followed immediately by infallible cleanup registration, before the pool is
/// returned. Return does not establish connectivity, authentication, query or
/// schema readiness: use a native operation or [`probe`] explicitly afterward.
/// Native pool options and callbacks are preserved without modification.
///
/// SQLx may start maintenance work during lazy construction, including minimum
/// connection work, so call this on a compatible live Tokio runtime. Native
/// constructor panics are not converted and no runtime-death cleanup is implied.
/// The finalizer awaits [`PgPool::close`]; callers must first join dependent work
/// and release checked-out connections. Local closure does not prove remote
/// cancellation, transaction rollback, or detached server-session termination.
///
/// ```no_run
/// use batter_core::cleanup::{CleanupBudget, CleanupStack};
/// use batter_sqlx::pool_in;
/// use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
/// use std::time::Duration;
/// # async fn example() -> Result<(), batter_core::RegistrationError> {
/// let mut cleanup = CleanupStack::new();
/// let pool = pool_in(
///     cleanup.reserve("postgres.pool")?,
///     PgPoolOptions::new(),
///     PgConnectOptions::new(),
/// );
/// // Perform an explicit query or `batter_sqlx::probe` before readiness.
/// # drop(pool);
/// # let second = Duration::from_secs(1);
/// # let budget = CleanupBudget::new(second, second, second).unwrap();
/// # assert!(cleanup.close(budget).await.is_success());
/// # Ok(()) }
/// ```
pub fn pool_in(
    slot: CleanupSlot<'_>,
    pool_options: PgPoolOptions,
    connect_options: PgConnectOptions,
) -> PgPool {
    let pool = pool_options.connect_lazy_with(connect_options);
    let closing = pool.clone();
    slot.register(move || async move {
        closing.close().await;
        Ok(())
    });
    pool
}

/// Register native pool close in the supervisor's explicit LIFO cleanup stack.
///
/// Register after acquisition, before dependents. On registration failure the
/// caller still owns `pool` and must close it explicitly. Cleanup is awaited
/// through the supervisor or extracted stack and bounded by its cleanup budget.
/// Closure waits for pool-accounted connections, not detached server sessions.
pub fn register_pool_close(
    supervisor: &mut Supervisor,
    name: &'static str,
    pool: &PgPool,
) -> Result<(), RegistrationError> {
    let pool = pool.clone();
    supervisor.on_cleanup(name, move || async move {
        pool.close().await;
        Ok(())
    })
}

#[cfg(feature = "test-support")]
pub mod test_support;
