//! Prune: deleting process rows past their own retention, then catalogs that
//! nothing references any more.
use crate::{PooledStore, StoreError, ValidationError};
use tokio::time::Instant;

/// The most rows one prune deletes from each table, 1 to 10 000.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PruneLimit(i64);

impl PruneLimit {
    /// Validates `rows`.
    pub fn new(rows: u16) -> Result<Self, ValidationError> {
        match rows {
            1..=10_000 => Ok(Self(i64::from(rows))),
            _ => Err(ValidationError),
        }
    }
}

/// What one prune deleted.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Pruned {
    /// Process rows whose expiry is older than their own retention.
    pub processes: u64,
    /// Catalogs unreferenced for 30 days that no process row references;
    /// their entries go with them.
    pub catalogs: u64,
}

impl PooledStore {
    /// Deletes up to `limit` expired process rows and then up to `limit`
    /// unreferenced catalogs, in one transaction. Every statement skips the
    /// rows another transaction holds, so a prune never waits on a heartbeat
    /// and a heartbeat that holds its catalog keeps it. A row is due only
    /// when its `expires_at` is older than the server clock minus the row's
    /// own `retention_seconds`; a pruner applies no retention of its own.
    pub async fn prune(&self, limit: PruneLimit, deadline: Instant) -> Result<Pruned, StoreError> {
        self.atomically(deadline, async |sql| {
            let processes = sqlx::query("DELETE FROM flags.flag_processes WHERE process_id IN (SELECT process_id FROM flags.flag_processes WHERE expires_at < clock_timestamp() - retention_seconds * interval '1 second' ORDER BY expires_at LIMIT $1 FOR UPDATE SKIP LOCKED)")
                .bind(limit.0).execute(sql.executor()).await?.rows_affected();
            let catalogs = sqlx::query("DELETE FROM flags.flag_catalogs WHERE manifest_digest IN (SELECT c.manifest_digest FROM flags.flag_catalogs c WHERE c.last_referenced_at < clock_timestamp() - interval '30 days' AND NOT EXISTS (SELECT FROM flags.flag_processes p WHERE p.manifest_digest = c.manifest_digest) ORDER BY c.last_referenced_at LIMIT $1 FOR UPDATE SKIP LOCKED)")
                .bind(limit.0).execute(sql.executor()).await?.rows_affected();
            Ok(Pruned { processes, catalogs })
        })
        .await
    }
}
