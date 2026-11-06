use crate::{
    PooledStore, StoreError,
    errors::{ErrorKind, check_server},
    store::{bounded, remaining},
};
use batter_core::operation::OperationOwner;
use batter_sqlx::{PgLease, PgReadOnlySnapshot};
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
    /// Batter lease. Allows later flags migrations while validating known checksums.
    pub async fn migrate(&self, deadline: Instant) -> Result<(), StoreError> {
        bounded(deadline, async {
            let version = bounded(deadline.min(Instant::now() + self.timing.acquire), async {
                PgReadOnlySnapshot::inspect(&self.pool, async |sql| {
                    sqlx::query_scalar::<_, i32>(
                        "SELECT current_setting('server_version_num')::integer",
                    )
                    .fetch_one(sql.executor())
                    .await
                    .map_err(StoreError::from)
                })
                .await
                .map_err(|e| StoreError(ErrorKind::Snapshot(Box::new(e))))
            })
            .await?;
            check_server(version)?;
            let owner = OperationOwner::new(remaining(deadline)?.min(self.timing.acquire))
                .map_err(|e| StoreError(ErrorKind::Other(Box::new(e))))?;
            let lease = PgLease::acquire(&self.pool, owner.context())
                .await
                .map_err(|e| match e {
                    batter_core::operation::OperationError::Failed(e) => {
                        StoreError(ErrorKind::Sql(e))
                    }
                    batter_core::operation::OperationError::Interrupted(_) => {
                        StoreError::classified(crate::ErrorClass::Deadline)
                    }
                })?;
            let mut migrator = sqlx::migrate!("./migrations");
            migrator.dangerous_set_table_name("flags._sqlx_migrations");
            migrator.create_schema("flags");
            migrator.set_ignore_missing(true);
            lease
                .migrate(&migrator)
                .await
                .map_err(|e| StoreError(ErrorKind::Sql(e)))
        })
        .await
    }
}
