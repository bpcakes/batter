//! Explicit client ownership for native SQLx PostgreSQL operations.
//!
//! [`PgLease`] retires on drop; only [`PgLease::return_to_pool`] opts into
//! ordinary SQLx pool return after application-acknowledged completion.
//! Retirement releases local pool capacity, **not** server locks or a confirmed
//! rollback. Detached server sessions can outlive [`PgPool::close`] and exceed
//! the pool's `max_connections`. Applications own remote outcome reconciliation.
//!
//! ```no_run
//! use batter::operation::OperationContext;
//! use batter_sqlx::{PgLease, SqlxFailure};
//! use sqlx::{Connection, PgPool};
//! # async fn example(pool: &PgPool, ctx: &OperationContext) -> Result<(), Box<dyn std::error::Error>> {
//! let mut lease = PgLease::acquire(pool, ctx).await?;
//! ctx.run("database.write", move |_| async move {
//!     let mut tx = lease.connection().begin().await.map_err(SqlxFailure::from)?;
//!     sqlx::query("SELECT 1").execute(&mut *tx).await.map_err(SqlxFailure::from)?;
//!     tx.commit().await.map_err(SqlxFailure::from)?;
//!     lease.return_to_pool(); // Commit acknowledgement, not a timeout inference.
//!     Ok::<_, SqlxFailure>(())
//! }).await?;
//! # Ok(()) }
//! ```
#![forbid(unsafe_code)]

mod failure;

pub use failure::{FailureClass, SqlxFailure};

use batter::{
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

/// One checked-out connection, retired unless explicitly returned after success.
///
/// Keep this owner **inside** the operation future so timeout, caller drop and
/// unwinding destroy it. A lease kept outside a cancelled future remains owned
/// until its caller drops it. Native transactions borrow this lease, so destroy
/// their futures/values before disposition. There is no asynchronous Drop,
/// transaction manager, automatic replay, or protection from process death.
#[must_use = "keep the lease inside the bounded operation and explicitly choose disposition"]
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

    /// Borrow the native connection for queries, streams or native transactions.
    ///
    /// Fully await successful query/commit/rollback before returning the lease.
    /// Dropping a query stream or transaction is not completion acknowledgement.
    pub fn connection(&mut self) -> &mut PgConnection {
        self.connection.as_mut().expect("live lease")
    }

    /// Opt into native SQLx pool return after acknowledged successful completion.
    ///
    /// SQLx asynchronously checks the connection before reuse and may discard it.
    /// This method neither waits for that check nor certifies connection state.
    /// Never call it after an interrupted query or uncertain commit. Await an
    /// explicit rollback instead of dropping an unfinished transaction when
    /// choosing reuse; if it was already dropped, retire this lease. Misuse can leave
    /// SQLx return waiting behind unfinished SQL.
    pub fn return_to_pool(mut self) {
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
            let mut lease = PgLease {
                connection: Some(connection),
            };
            sqlx::query("SELECT 1")
                .execute(lease.connection())
                .await
                .map_err(SqlxFailure::from)?;
            lease.return_to_pool();
            Ok(())
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
/// use batter::cleanup::{CleanupBudget, CleanupStack};
/// use batter_sqlx::pool_in;
/// use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
/// use std::time::Duration;
/// # async fn example() -> Result<(), batter::RegistrationError> {
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
