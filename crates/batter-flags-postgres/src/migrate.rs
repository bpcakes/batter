use crate::{
    PooledStore, StoreError,
    atomic::scope_error,
    errors::{ErrorKind, check_server},
    store::{bounded, remaining},
    write::RollbackFailure,
};
use batter_core::operation::OperationOwner;
use batter_sqlx::{PgDeadlineError, PgLease, PgMigrationTimeouts, low_level::PgAtomicTransaction};
use sqlx::migrate::Migrator;
use std::sync::LazyLock;
use tokio::time::Instant;

/// First immutable flags migration, for application vendoring.
pub const CREATE_FLAGS_SQL: &str = include_str!("../migrations/20261106000000_create_flags.sql");
/// Schema revision needed by this store. Newer revisions remain readable.
pub const REQUIRED_SCHEMA_REVISION: i32 = 1;
/// Strict migrator using only `flags._sqlx_migrations`, independent of host ledgers.
/// Low-level consumers own disposition; prefer [`PooledStore::migrate`].
pub static MIGRATOR: LazyLock<Migrator> = LazyLock::new(|| {
    let mut migrator = sqlx::migrate!("./migrations");
    migrator.dangerous_set_table_name("flags._sqlx_migrations");
    migrator.create_schema("flags");
    migrator
});
/// Where the store's own session resolves unqualified names once the `flags`
/// schema exists. The probe makes it exist, so there is no "cannot tell".
enum Resolution {
    /// The effective search path does not include the `flags` schema.
    Elsewhere,
    /// It does, so unqualified host names, SQLx's default
    /// `_sqlx_migrations` among them, can resolve into `flags`.
    IntoFlags,
}

/// The store's session, as one rolled-back probe sees it.
struct Session {
    version: i32,
    resolution: Resolution,
}

impl PooledStore {
    /// Refuse older servers, then run the flags-only ledger through a retiring
    /// Batter lease with server timeouts. Allows later flags migrations while validating known checksums.
    ///
    /// Refuses with [`ErrorClass::SearchPath`](crate::ErrorClass::SearchPath)
    /// when a session of the store's pool resolves unqualified names into the
    /// `flags` schema, before anything is migrated. PostgreSQL's
    /// `current_schemas` lists only existing schemas, so the probe creates the
    /// schema if it is absent, inside a transaction that it always rolls
    /// back, and lets PostgreSQL resolve the session's default path against
    /// it; a first migration is judged exactly like a later one. The probe
    /// needs only the database `CREATE` privilege SQLx's own
    /// `CREATE SCHEMA IF NOT EXISTS` needs on every run.
    ///
    /// The probe and the migration use separate connections: those of one
    /// pool share their default path unless the login's role or database
    /// settings or the pool's connect options changed while it held
    /// connections, which the check cannot see. Sessions of other pools or
    /// logins are the application's to configure.
    ///
    /// A server, schema-revision or search-path refusal applies nothing. A
    /// failure while migrating can follow committed migrations, whatever its
    /// class, because SQLx commits each migration on its own; a rerun applies
    /// nothing twice.
    pub async fn migrate(&self, deadline: Instant) -> Result<(), StoreError> {
        let session = self.inspect_session(deadline).await?;
        check_server(session.version)?;
        refuse_flags_path(&session.resolution)?;
        let owner = OperationOwner::new(remaining(deadline)?.min(self.timing.acquire))
            .map_err(|e| StoreError(ErrorKind::Other(Box::new(e))))?;
        let lease = PgLease::acquire(&self.pool, owner.context())
            .await
            .map_err(|e| match e {
                batter_core::operation::OperationError::Failed(e) => StoreError(ErrorKind::Sql(e)),
                batter_core::operation::OperationError::Interrupted(_) => {
                    StoreError::classified(crate::ErrorClass::Deadline)
                }
            })?;
        let mut migrator = sqlx::migrate!("./migrations");
        migrator.dangerous_set_table_name("flags._sqlx_migrations");
        migrator.create_schema("flags");
        migrator.set_ignore_missing(true);
        let limits =
            PgMigrationTimeouts::new(self.timing.statement, self.timing.lock, self.timing.idle)
                .map_err(|e| StoreError(ErrorKind::Other(Box::new(e))))?;
        lease
            .migrate_until(&migrator, limits, deadline)
            .await
            .map_err(|e| match e {
                PgDeadlineError::Elapsed => StoreError::classified(crate::ErrorClass::Deadline),
                PgDeadlineError::Failed(e) => e.into(),
                PgDeadlineError::RecoveryElapsed(never) => match never {},
            })
    }

    /// Reads the server version and where this pool's sessions resolve
    /// unqualified names once a `flags` schema exists, in one transaction
    /// that is always rolled back, so the schema it may create is never
    /// committed.
    async fn inspect_session(&self, deadline: Instant) -> Result<Session, StoreError> {
        let deadline = deadline.min(Instant::now() + self.timing.acquire);
        let tx = bounded(deadline, async {
            PgAtomicTransaction::begin(&self.pool)
                .await
                .map_err(StoreError::from)
        })
        .await?;
        let (tx, probed) = tx
            .operation_until(deadline, async |sql| {
                self.limit(sql, deadline).await?;
                sqlx::query("SET LOCAL search_path = pg_catalog, pg_temp")
                    .execute(sql.executor())
                    .await?;
                let version = sqlx::query_scalar::<_, i32>(
                    "SELECT pg_catalog.current_setting('server_version_num')::integer",
                )
                .fetch_one(sql.executor())
                .await
                .map_err(StoreError::from)?;
                // `current_schemas` names only schemas that exist, so make
                // `flags` exist here: the transaction rolls back below, and
                // a schema that already exists is left as it is.
                sqlx::query("CREATE SCHEMA IF NOT EXISTS flags")
                    .execute(sql.executor())
                    .await?;
                // Acquisition ran DISCARD ALL, so the session default
                // `reset_val` is the path, and `current_user` the login, that
                // this pool's sessions use; sessions of one pool share them
                // unless role, database or connect-option defaults changed
                // while the pool held connections. Let PostgreSQL resolve it
                // (quoting, `"$user"`, existing schemas only) for one fully
                // qualified statement, then restore the hardened path.
                sqlx::query(
                    "SELECT pg_catalog.set_config('search_path', reset_val, true) \
                     FROM pg_catalog.pg_settings WHERE name OPERATOR(pg_catalog.=) 'search_path'",
                )
                .execute(sql.executor())
                .await?;
                let into_flags = sqlx::query_scalar::<_, bool>(
                    "SELECT 'flags'::pg_catalog.name OPERATOR(pg_catalog.=) \
                     ANY (pg_catalog.current_schemas(false))",
                )
                .fetch_one(sql.executor())
                .await
                .map_err(StoreError::from)?;
                sqlx::query(
                    "SELECT pg_catalog.set_config('search_path', 'pg_catalog, pg_temp', true)",
                )
                .execute(sql.executor())
                .await?;
                let resolution = if into_flags {
                    Resolution::IntoFlags
                } else {
                    Resolution::Elsewhere
                };
                Ok(Session {
                    version,
                    resolution,
                })
            })
            .await
            .map_err(scope_error)?;
        // Roll back everything the probe did, a schema it created included.
        // If the rollback cannot be confirmed in time, dropping the owner
        // retires its connection, which ends the transaction uncommitted.
        let rolled_back = bounded(deadline, async {
            tx.rollback().await.map_err(StoreError::from)
        })
        .await;
        match (probed, rolled_back) {
            (Ok(session), Ok(_)) => Ok(session),
            (Err(error), Ok(_)) => Err(error),
            (Ok(_), Err(cleanup)) => Err(cleanup),
            (Err(error), Err(cleanup)) => Err(RollbackFailure::both(error, cleanup)),
        }
    }
}

fn refuse_flags_path(resolution: &Resolution) -> Result<(), StoreError> {
    match resolution {
        Resolution::Elsewhere => Ok(()),
        Resolution::IntoFlags => Err(StoreError::classified(crate::ErrorClass::SearchPath)),
    }
}
