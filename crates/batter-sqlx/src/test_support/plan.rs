//! Declared fixture capacity and the login each managed pool authenticates as.

use std::{
    fmt,
    sync::atomic::{AtomicU64, Ordering},
};

use batter_core::settings::SecretString;
use sqlx::{
    PgPool,
    postgres::{PgConnectOptions, PgPoolOptions},
};

use super::FixtureError;

// Plan identities only distinguish declarations; they carry no authority.
static NEXT_PLAN: AtomicU64 = AtomicU64::new(1);

/// Declared application capacity for one lease, validated before acquisition.
///
/// Reserved connections include standalone blockers, observers and administration
/// even when they target the admin database. Separately bounded suite resources
/// and upstream owner/template/lifecycle sessions are additional server usage.
/// Multiple harnesses do not share a server-wide budget. Pool clones share
/// capacity; every separately constructed pool needs its own declaration.
///
/// Each acquired lease resolves every declared pool's native options from the
/// lease URL, then constructs and registers every pool in declaration order
/// before the first connectivity check. A registered pool is not a connected
/// one: pools without eager `min_connections` open nothing until used.
#[derive(Clone)]
pub struct ConnectionPlan {
    pools: Vec<PlannedPool>,
    total: u32,
    id: u64,
}

impl ConnectionPlan {
    /// Sum pool maxima and standalone capacity using checked arithmetic.
    ///
    /// Every pool authenticates with the lease URL's own login, so positional
    /// `pools()` access is unambiguous. Use [`Self::declare`] for pools that
    /// authenticate as a separately provisioned login.
    pub fn new(pools: Vec<PgPoolOptions>, reserved: u32) -> Result<Self, FixtureError> {
        let mut declaration = Self::declare(reserved);
        for options in pools {
            declaration.pool(PlannedPool::lease_login(options));
        }
        declaration.finish()
    }

    /// Start a declaration whose pools may authenticate as different logins.
    ///
    /// Each [`PlanDeclaration::pool`] returns a [`PoolKey`] that later selects
    /// that pool from the acquired database; selecting by key cannot return
    /// another declared pool or one from another plan. Positional `pools()`
    /// access remains for inspecting every pool, but an index does not record
    /// which login a pool uses. Capacity rules match [`Self::new`].
    ///
    /// ```no_run
    /// use batter_core::settings::SecretString;
    /// use batter_sqlx::test_support::{ConnectionPlan, FixtureError, FixtureSuite, PlannedPool};
    /// use sqlx::postgres::PgPoolOptions;
    /// # async fn example(suite: FixtureSuite, role: String, password: SecretString)
    /// # -> Result<(), Box<dyn std::error::Error>> {
    /// // The outer test owner created `role` (LOGIN, fresh password, VALID UNTIL)
    /// // before the run and drops it after inspecting the report.
    /// let run = suite.start(move |scope| Box::pin(async move {
    ///     let mut plan = ConnectionPlan::declare(0);
    ///     let owner = plan.pool(PlannedPool::lease_login(PgPoolOptions::new().max_connections(1)));
    ///     let login = plan.pool(PlannedPool::password_login(
    ///         PgPoolOptions::new().max_connections(1), role.clone(), password));
    ///     let plan = plan.finish()?;
    ///     let db = scope.empty(&plan).await?;
    ///     let (owner, restricted) = (db.pool(owner).unwrap(), db.pool(login).unwrap());
    ///     let session: String = sqlx::query_scalar("SELECT session_user::text")
    ///         .fetch_one(restricted).await.map_err(FixtureError::Observe)?;
    ///     assert_eq!(session, role);
    ///     sqlx::query("CREATE TABLE item (id bigint)")
    ///         .execute(owner).await.map_err(FixtureError::Observe)?;
    ///     let grant: String = sqlx::query_scalar("SELECT format('GRANT SELECT ON item TO %I', $1::text)")
    ///         .bind(&role).fetch_one(owner).await.map_err(FixtureError::Observe)?;
    ///     sqlx::query(sqlx::AssertSqlSafe(grant))
    ///         .execute(owner).await.map_err(FixtureError::Observe)?;
    ///     sqlx::query("SELECT id FROM item")
    ///         .fetch_all(restricted).await.map_err(FixtureError::Observe)?;
    ///     Ok::<_, FixtureError>(())
    /// }));
    /// run.into_report().await?.into_result()?;
    /// # Ok(()) }
    /// ```
    pub fn declare(reserved: u32) -> PlanDeclaration {
        PlanDeclaration {
            id: NEXT_PLAN.fetch_add(1, Ordering::Relaxed),
            pools: Vec::new(),
            reserved,
        }
    }

    pub(super) fn id(&self) -> u64 {
        self.id
    }

    /// Reject a declaration larger than the selected per-database capacity.
    pub fn validate(&self, per_database: u32) -> Result<(), FixtureError> {
        if self.total > per_database {
            Err(FixtureError::InvalidBudget)
        } else {
            Ok(())
        }
    }

    /// Resolve every declared pool's native options for one lease. Pure: no
    /// pool is constructed, no connection is attempted and no caller code runs.
    pub(super) fn resolve(
        &self,
        lease_url: &str,
        lease_name: &str,
    ) -> Result<Vec<(PgPoolOptions, PgConnectOptions)>, sqlx::Error> {
        let lease: PgConnectOptions = lease_url.parse()?;
        // SQLx lets a `dbname` query parameter override the URL path, and the
        // harness keeps the admin URL's query when it builds the lease URL.
        if lease.get_database() != Some(lease_name) {
            return Err(sqlx::Error::Configuration(Box::new(LeaseDatabaseMismatch)));
        }
        Ok(self
            .pools
            .iter()
            .map(|pool| (pool.options.clone(), pool.login.apply(lease.clone())))
            .collect())
    }
}

/// An unfinished [`ConnectionPlan`] that hands out a [`PoolKey`] per pool.
#[must_use = "finish the declaration to obtain a ConnectionPlan"]
pub struct PlanDeclaration {
    id: u64,
    pools: Vec<PlannedPool>,
    reserved: u32,
}

impl PlanDeclaration {
    /// Declare one pool and return the key that selects it after acquisition.
    pub fn pool(&mut self, pool: PlannedPool) -> PoolKey {
        self.pools.push(pool);
        PoolKey {
            plan: self.id,
            index: self.pools.len() - 1,
        }
    }

    /// Validate the declaration: at least one pool, a positive maximum for each,
    /// and a checked sum of maxima plus reserved capacity.
    pub fn finish(self) -> Result<ConnectionPlan, FixtureError> {
        let total = self
            .pools
            .iter()
            .try_fold(self.reserved, |total, pool| {
                total.checked_add(pool.options.get_max_connections())
            })
            .ok_or(FixtureError::InvalidBudget)?;
        if self.pools.is_empty()
            || self
                .pools
                .iter()
                .any(|pool| pool.options.get_max_connections() == 0)
        {
            return Err(FixtureError::InvalidBudget);
        }
        Ok(ConnectionPlan {
            pools: self.pools,
            total,
            id: self.id,
        })
    }
}

/// Opaque handle to one declared pool, valid only for databases acquired with
/// the plan (or a clone of the plan) that issued it. A key from another plan
/// selects nothing rather than a different pool.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PoolKey {
    plan: u64,
    index: usize,
}

/// Select a pool by key, refusing keys issued by a different plan.
pub(super) fn lookup(pools: &[PgPool], plan: u64, key: PoolKey) -> Option<&PgPool> {
    if key.plan == plan {
        pools.get(key.index)
    } else {
        None
    }
}

/// One declared native pool and the login it authenticates as.
///
/// Both logins keep everything else from the acquired lease URL: server
/// address, socket, database, TLS mode and certificates (including values SQLx
/// reads from `PG*` environment variables), `application_name`, startup options
/// and statement cache. Pool capacity, timeouts and the native session hooks
/// (`after_connect`, `before_acquire`, `after_release`) come from `options`.
/// Neither constructor can select another server or database.
///
/// The lease binding holds when the pool is constructed. Native
/// `PgPool::set_connect_options` can still retarget later connections and
/// `PgPool::connect_options` reveals credentials; never Debug-format native
/// connect options, which include the password.
#[derive(Clone)]
pub struct PlannedPool {
    options: PgPoolOptions,
    login: Login,
}

#[derive(Clone)]
enum Login {
    Lease,
    Password {
        username: String,
        password: SecretString,
    },
}

impl Login {
    fn apply(&self, lease: PgConnectOptions) -> PgConnectOptions {
        match self {
            Self::Lease => lease,
            Self::Password { username, password } => {
                lease.username(username).password(password.expose_secret())
            }
        }
    }
}

impl PlannedPool {
    /// Authenticate with the lease URL's own credentials: the harness admin
    /// login that owns the disposable database. `ConnectionPlan::new` uses this.
    pub fn lease_login(options: PgPoolOptions) -> Self {
        Self {
            options,
            login: Login::Lease,
        }
    }

    /// Authenticate as a separately provisioned PostgreSQL login.
    ///
    /// Only the username and password differ from the lease. The lease owner's
    /// password, whether from the URL, `PGPASSWORD` or a passfile, is always
    /// replaced, and no passfile is consulted for this login. The password is a
    /// [`SecretString`] so it cannot be swapped with the username, which
    /// PostgreSQL echoes into authentication errors and server logs.
    /// Client-certificate logins are not supported. An empty or unknown username
    /// is rejected by the server at the connectivity check, not here. The
    /// password is only checked when the server's `pg_hba.conf` rule for the
    /// lease's transport uses password authentication; a `trust` rule accepts
    /// any password, so credential tests should assert `system_user`.
    ///
    /// The caller provisions the role (LOGIN, password, a bounded VALID UNTIL)
    /// before the run, grants what each test needs and drops the role after
    /// inspecting the report; role lifecycle is cluster-scoped and stays outside
    /// fixture cleanup. Put session policy in `options`' native hooks, for example
    /// `PgSessionProfile::reset_and_apply` in `after_connect` and `after_release`.
    /// `PgProfiledPool` and `RunledgerDatabase` constructors cannot host fixture
    /// pools. Lease cleanup forcibly drops the database, which can terminate a
    /// lingering restricted session only when the harness admin login may signal
    /// it (superuser, member of the role, or `pg_signal_backend`).
    pub fn password_login(
        options: PgPoolOptions,
        username: impl Into<String>,
        password: SecretString,
    ) -> Self {
        Self {
            options,
            login: Login::Password {
                username: username.into(),
                password,
            },
        }
    }
}

/// The acquired lease URL selects a database other than the lease's own.
///
/// Carried as the source of `sqlx::Error::Configuration` in
/// `FixtureError::PoolAcquire`, `FixtureError::Connect` and
/// `DatabaseCleanup::pool_failures`. It is raised before any pool is
/// constructed. With `PoolAcquire` the lease stays registered for driver
/// cleanup and the final report records the outcome; with `Connect` lease
/// cleanup was already awaited and its outcome is in `cleanup`. SQLx applies a
/// `dbname` query parameter over the URL path, so a harness admin URL carrying
/// one causes this. Formatting names no database.
///
/// ```
/// # fn example(error: &sqlx::Error) -> bool {
/// use batter_sqlx::test_support::LeaseDatabaseMismatch;
/// matches!(error, sqlx::Error::Configuration(source) if source.is::<LeaseDatabaseMismatch>())
/// # }
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct LeaseDatabaseMismatch;

impl fmt::Display for LeaseDatabaseMismatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("fixture lease URL does not select the leased database")
    }
}

impl std::error::Error for LeaseDatabaseMismatch {}

/// Construct each resolved pool and register it before constructing the next,
/// so a construction panic never leaves an earlier pool unregistered.
pub(super) fn construct(
    resolved: Vec<(PgPoolOptions, PgConnectOptions)>,
    registry: &mut Vec<PgPool>,
) -> Vec<PgPool> {
    let mut pools = Vec::with_capacity(resolved.len());
    for (options, connect) in resolved {
        let pool = options.connect_lazy_with(connect);
        registry.push(pool.clone());
        pools.push(pool);
    }
    pools
}

/// Check one connection per pool in declaration order, stopping at the first
/// failure. SQLx runs `after_connect` inside this cancellable acquisition.
pub(super) async fn check(pools: &[PgPool]) -> Result<(), sqlx::Error> {
    for pool in pools {
        drop(pool.acquire().await?);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
