//! Shared pool/profile ownership for native adapters and owned atomic work.
use crate::PgSessionProfile;
use sqlx::{
    ConnectOptions, PgPool,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use std::sync::Arc;

/// A pool with one immutable declared session profile and mandatory reset hooks.
/// No arbitrary-pool constructor or replaceable setup callback is supported.
/// All three hooks are owned here, including restoration before idle admission
/// for native `try_acquire`/`try_begin` paths that skip acquisition hooks.
///
/// Profiles describe session policy, not endpoint identity or permanent grants.
/// Native SQL remains an escape hatch, not a sandbox or atomic outcome guarantee.
/// Use the same owner's [`Self::profile`] for profiled owned transactions.
///
/// ```no_run
/// # async fn example(options: sqlx::postgres::PgConnectOptions,
/// # profile: batter_sqlx::PgSessionProfile) -> Result<(), sqlx::Error> {
/// let database = batter_sqlx::PgProfiledPool::connect(options, profile, 4).await?;
/// let role: String = sqlx::query_scalar("SELECT current_user::text")
///     .fetch_one(database.pool()).await?;
/// # let _ = role;
/// database.pool().close().await;
/// # Ok(()) }
/// ```
#[derive(Clone)]
pub struct PgProfiledPool {
    pool: PgPool,
    profile: Arc<PgSessionProfile>,
}

impl std::fmt::Debug for PgProfiledPool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PgProfiledPool")
    }
}

impl PgProfiledPool {
    /// Connect under explicit policy. Roles/schemas must already exist. A direct
    /// probe preserves configuration errors instead of a pool retry timeout;
    /// it never returns to the pool, including on cancellation. Zero capacity
    /// is rejected. Profile setup failures have redacted default formatting;
    /// connection errors remain native SQLx errors. See
    /// [`PgSessionProfile::reset_and_apply`] for logging limits.
    pub async fn connect(
        options: PgConnectOptions,
        profile: PgSessionProfile,
        max_connections: u32,
    ) -> Result<Self, sqlx::Error> {
        validate_capacity(max_connections)?;
        let mut probe = options.connect().await?;
        profile.reset_and_apply(&mut probe).await?;
        drop(probe);
        let database = Self::connect_lazy(
            options,
            profile,
            PgPoolOptions::new().max_connections(max_connections),
        )?;
        drop(database.pool.acquire().await?);
        Ok(database)
    }

    /// Construct a lazy pool, preserving native capacity/lifetime configuration
    /// but replacing all session hooks with the declared policy. SQLx may start
    /// minimum-connection maintenance: reserve cleanup before construction and
    /// register pool close before yielding. This does not verify connectivity.
    ///
    /// Returned sessions are reset and verified before becoming idle. Failed
    /// restoration discards the connection; native `try_*` calls may return
    /// `None` while asynchronous release cleanup is running. No session hook
    /// may be used to add application authority outside the explicit profile.
    pub fn connect_lazy(
        options: PgConnectOptions,
        profile: PgSessionProfile,
        pool_options: PgPoolOptions,
    ) -> Result<Self, sqlx::Error> {
        validate_capacity(pool_options.get_max_connections())?;
        let profile = Arc::new(profile);
        let on_connect = Arc::clone(&profile);
        let on_acquire = Arc::clone(&profile);
        let on_release = Arc::clone(&profile);
        let pool = pool_options
            .after_connect(move |connection, _| {
                let profile = Arc::clone(&on_connect);
                Box::pin(async move { profile.reset_and_apply(connection).await })
            })
            .before_acquire(move |connection, _| {
                let profile = Arc::clone(&on_acquire);
                Box::pin(async move { profile.reset_and_apply(connection).await.map(|()| true) })
            })
            .after_release(move |connection, _| {
                let profile = Arc::clone(&on_release);
                // Fast native acquisitions skip before_acquire. Normalize before
                // publishing idle sessions; SQLx closes the connection on Err.
                Box::pin(async move { profile.reset_and_apply(connection).await.map(|()| true) })
            })
            .connect_lazy_with(options);
        Ok(Self { pool, profile })
    }

    /// Native access under the declared profile. Raw SQL can change its current
    /// session; normalization occurs before subsequent acquisition/idle reuse.
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// Immutable declared policy, shared with profiled owned scopes.
    pub fn profile(&self) -> &PgSessionProfile {
        &self.profile
    }
}

fn validate_capacity(capacity: u32) -> Result<(), sqlx::Error> {
    if capacity == 0 {
        return Err(sqlx::Error::Protocol(
            "profiled pool capacity must be positive".into(),
        ));
    }
    Ok(())
}
