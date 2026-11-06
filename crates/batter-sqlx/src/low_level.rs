//! Explicit caller-owned pool and transaction composition.
//!
//! Prefer [`crate::pool_in`] and [`crate::run_atomic`]. Standalone native callers
//! own registration, cancellation and awaited pool closure.
//! Manual transaction outputs remain provisional until caller-paired completion.
//! Pool closure waits for checkouts; it does not prove detached server-session termination.
use crate::{PgProfiledPool, PgSessionProfile};
use batter_core::{RegistrationError, cleanup::CleanupSlot, lifecycle::Supervisor};
use sqlx::{
    PgPool,
    postgres::{PgConnectOptions, PgPoolOptions},
};

pub use crate::atomic::PgAtomicTransaction;

/// Connect a caller-owned profiled pool, probing configuration directly.
/// Migration: `PgProfiledPool::connect` moved here. Prefer [`crate::pool_in`]
/// inside protected startup; this function installs no finalizer. The caller
/// must close the pool after dependents stop, including on initialization failure.
pub async fn connect_profiled_pool(
    options: PgConnectOptions,
    profile: PgSessionProfile,
    max_connections: u32,
) -> Result<PgProfiledPool, sqlx::Error> {
    PgProfiledPool::connect(options, profile, max_connections).await
}

/// Construct a caller-owned lazy profiled pool with mandatory profile hooks.
/// Migration: `PgProfiledPool::connect_lazy` moved here. Prefer [`crate::pool_in`].
/// SQLx may start maintenance immediately; reserve cleanup before calling and
/// register closure without yielding. No connectivity or readiness is proved.
pub fn connect_profiled_pool_lazy(
    options: PgConnectOptions,
    profile: PgSessionProfile,
    pool_options: PgPoolOptions,
) -> Result<PgProfiledPool, sqlx::Error> {
    PgProfiledPool::connect_lazy(options, profile, pool_options)
}

/// Construct an unprofiled native pool, preserving arbitrary caller-owned hooks.
/// Migration: the old three-argument `batter_sqlx::pool_in` is now
/// `low_level::pool_in`. Prefer [`crate::pool_in`] with an explicit profile.
/// Native session authority remains caller policy on this escape path.
///
/// `slot` must be reserved before this call. Successful native construction is
/// followed immediately by infallible cleanup registration, before the pool is
/// returned. Return does not establish connectivity, authentication, query or
/// schema readiness: use a native operation or [`crate::probe`] explicitly afterward.
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
/// use batter_sqlx::low_level::pool_in;
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

/// Migration: `batter_sqlx::register_pool_close` moved into `low_level`.
/// Prefer [`crate::pool_in`] to eliminate the acquisition/registration gap.
///
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
