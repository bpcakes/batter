use crate::{
    PooledStore, StoreError,
    errors::{ErrorKind, check_server},
    store::remaining,
};
use batter_core::operation::OperationOwner;
use batter_sqlx::{PgDeadlineError, PgLease, PgMigrationTimeouts, PgReadOnlySnapshot};
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
/// Where the store's own session resolves unqualified names.
enum Resolution {
    /// The effective search path does not include the `flags` schema.
    Elsewhere,
    /// It does, so unqualified host names, SQLx's default
    /// `_sqlx_migrations` among them, can resolve into `flags`.
    IntoFlags,
}

/// The store's session, as one read-only inspection sees it.
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
    /// `flags` schema. PostgreSQL's `current_schemas` lists only existing
    /// schemas, so one session is checked before migrating, when the schema
    /// already exists, and another after. These are separate connections:
    /// those of one pool share their default path unless the login's role or
    /// database settings or the pool's connect options changed while it held
    /// connections, which the check cannot see. Sessions of other
    /// pools or logins are the application's to configure.
    ///
    /// Any failure can follow committed migrations, whatever its class: SQLx
    /// commits each migration on its own, and the check after migrating can
    /// itself refuse, fail or run out of time once they have committed. A
    /// rerun applies nothing twice.
    pub async fn migrate(&self, deadline: Instant) -> Result<(), StoreError> {
        let before = self.inspect_session(deadline).await?;
        check_server(before.version)?;
        refuse_flags_path(&before.resolution)?;
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
            })?;
        // The schema exists now, so this sees a path that resolves into it
        // even on the first migration.
        let after = self.inspect_session(deadline).await?;
        refuse_flags_path(&after.resolution)
    }

    /// Reads the server version and where this session resolves unqualified
    /// names, in one bounded read-only snapshot.
    async fn inspect_session(&self, deadline: Instant) -> Result<Session, StoreError> {
        let deadline = deadline.min(Instant::now() + self.timing.acquire);
        PgReadOnlySnapshot::inspect_until(&self.pool, deadline, async |sql| {
            let [statement, lock, idle] = self.timing.settings(deadline)?;
            sqlx::query("SELECT set_config('statement_timeout',$1,true), set_config('lock_timeout',$2,true), set_config('idle_in_transaction_session_timeout',$3,true)")
                .bind(statement).bind(lock).bind(idle).execute(sql.executor()).await?;
            let version = sqlx::query_scalar::<_, i32>(
                "SELECT current_setting('server_version_num')::integer",
            )
            .fetch_one(sql.executor())
            .await
            .map_err(StoreError::from)?;
            // The snapshot hardens its own path to `pg_catalog, pg_temp`.
            // Acquisition ran DISCARD ALL, so the session default `reset_val`
            // is the path, and `current_user` the login, that this session's
            // statements outside the snapshot use; sessions of one pool share
            // them unless role, database or connect-option defaults changed
            // while the pool held connections.
            // Let PostgreSQL resolve it (quoting, `"$user"`, existing schemas
            // only) for one fully qualified statement, then restore the
            // hardened path.
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
            sqlx::query("SELECT pg_catalog.set_config('search_path', 'pg_catalog, pg_temp', true)")
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
        .map_err(crate::load::snapshot_error)
    }
}

fn refuse_flags_path(resolution: &Resolution) -> Result<(), StoreError> {
    match resolution {
        Resolution::Elsewhere => Ok(()),
        Resolution::IntoFlags => Err(StoreError::classified(crate::ErrorClass::SearchPath)),
    }
}
