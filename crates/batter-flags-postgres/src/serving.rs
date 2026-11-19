//! The session owner for refresh, presence and pruning.
use crate::{Loaded, PooledStore, ProcessPresence, PruneLimit, Pruned, StoreError, StoreTiming};
use batter_sqlx::{
    PgProfiledPool, PgReadOnlySnapshot, PgReadOnlySql, PgSessionProfile,
    low_level::PgAtomicTransaction,
};
use tokio::time::Instant;

#[derive(Clone, Debug)]
enum Session {
    Profiled(PgProfiledPool),
    Raw(PooledStore),
}

/// The serving operations over one pool and its declared policy. Every
/// checkout re-establishes the profile before flags SQL. Store timeouts can
/// shorten declared limits. The serving capability cannot run migrations.
///
/// ```no_run
/// # async fn example(pool: &batter_sqlx::PgProfiledPool, timing: batter_flags_postgres::StoreTiming, deadline: tokio::time::Instant) -> Result<(), batter_flags_postgres::StoreError> {
/// let store = batter_flags_postgres::ServingStore::new(pool, timing);
/// let snapshot = store.load(deadline).await?;
/// # Ok(()) }
/// ```
#[derive(Clone, Debug)]
pub struct ServingStore {
    session: Session,
    pub(crate) timing: StoreTiming,
}

impl ServingStore {
    /// Bind to the pool's own immutable session policy.
    pub fn new(pool: &PgProfiledPool, timing: StoreTiming) -> Self {
        Self {
            session: Session::Profiled(pool.clone()),
            timing,
        }
    }

    /// The timing used by this process.
    pub fn timing(&self) -> StoreTiming {
        self.timing
    }

    /// Mint a presence that uses the same serving policy for every heartbeat and stop.
    pub fn presence(
        &self,
        labels: crate::ProcessLabels,
        base: &batter_flags_core::low_level::Base,
        cadence: crate::Cadence,
    ) -> ProcessPresence {
        ProcessPresence::for_serving(self.clone(), labels, base, cadence)
    }

    fn profile(
        &self,
        pool: &PgProfiledPool,
        deadline: Instant,
    ) -> Result<PgSessionProfile, StoreError> {
        let [statement, lock, idle] = self.timing.caps(deadline)?;
        pool.profile()
            .with_timeout_caps(statement, lock, idle)
            .map_err(|e| StoreError(crate::errors::ErrorKind::Other(Box::new(e))))
    }

    pub(crate) async fn inspect<T>(
        &self,
        deadline: Instant,
        inspect: impl AsyncFnOnce(&mut PgReadOnlySql<'_>) -> Result<T, StoreError>,
    ) -> Result<T, StoreError> {
        match &self.session {
            Session::Raw(store) => PgReadOnlySnapshot::inspect_until(&store.pool, deadline, async |sql| {
                let [statement, lock, idle] = self.timing.settings(deadline)?;
                sqlx::query("SELECT set_config('statement_timeout',$1,true), set_config('lock_timeout',$2,true), set_config('idle_in_transaction_session_timeout',$3,true)")
                    .bind(statement).bind(lock).bind(idle).execute(sql.executor()).await?;
                store.refuse_standby(sql).await?;
                inspect(sql).await
            }).await,
            Session::Profiled(pool) => {
                let profile = self.profile(pool, deadline)?;
                PgReadOnlySnapshot::inspect_profiled_until(pool.pool(), &profile, deadline, async |sql| {
                    crate::store::require_primary(sql).await?;
                    inspect(sql).await
                }).await
            }
        }.map_err(crate::load::snapshot_error)
    }

    pub(crate) async fn begin(&self, deadline: Instant) -> Result<PgAtomicTransaction, StoreError> {
        Ok(match &self.session {
            Session::Raw(store) => PgAtomicTransaction::begin(&store.pool).await?,
            Session::Profiled(pool) => {
                PgAtomicTransaction::begin_profiled(pool.pool(), &self.profile(pool, deadline)?)
                    .await?
            }
        })
    }

    pub(crate) async fn limit_raw(
        &self,
        sql: &mut batter_sqlx::PgScopedSql<'_>,
        deadline: Instant,
    ) -> Result<(), StoreError> {
        match &self.session {
            Session::Raw(store) => store.limit(sql, deadline).await,
            Session::Profiled(_) => Ok(()),
        }
    }
}

impl PooledStore {
    pub(crate) fn serving(&self) -> ServingStore {
        ServingStore {
            session: Session::Raw(self.clone()),
            timing: self.timing,
        }
    }

    /// Read the primary's version and unarchived rows in one bounded snapshot.
    pub async fn load(&self, deadline: Instant) -> Result<Loaded, StoreError> {
        self.serving().load(deadline).await
    }

    /// Prune expired presence and catalogs under their retention rules.
    pub async fn prune(&self, limit: PruneLimit, deadline: Instant) -> Result<Pruned, StoreError> {
        self.serving().prune(limit, deadline).await
    }
}
