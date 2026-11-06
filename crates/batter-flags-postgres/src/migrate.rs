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
impl PooledStore {
    /// Refuse older servers, then run the flags-only ledger through a retiring
    /// Batter lease with server timeouts. Allows later flags migrations while validating known checksums.
    pub async fn migrate(&self, deadline: Instant) -> Result<(), StoreError> {
        let preflight_deadline = deadline.min(Instant::now() + self.timing.acquire);
        let version = PgReadOnlySnapshot::inspect_until(&self.pool, preflight_deadline, async |sql| {
                    let [statement, lock, idle] = self.timing.settings(preflight_deadline)?;
                    sqlx::query("SELECT set_config('statement_timeout',$1,true), set_config('lock_timeout',$2,true), set_config('idle_in_transaction_session_timeout',$3,true)")
                        .bind(statement).bind(lock).bind(idle).execute(sql.executor()).await?;
                    sqlx::query_scalar::<_, i32>(
                        "SELECT current_setting('server_version_num')::integer",
                    )
                    .fetch_one(sql.executor())
                    .await
                    .map_err(StoreError::from)
                })
                .await
                .map_err(crate::load::snapshot_error)?;
        check_server(version)?;
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
}
