use crate::{PooledStore, StoreError, StoredState, errors::ErrorKind, store::bounded};
use batter_flags_core::{FlagKey, Version};
use batter_sqlx::PgReadOnlySnapshot;
use sqlx::Row;
use tokio::time::Instant;

/// One retained audit event. Actor, reason and state are deliberately explicit fields.
/// Default formatting discloses only the counter.
pub struct HistoryEntry {
    /// Commit-ordered counter.
    pub version: Version,
    /// Affected key.
    pub key: String,
    /// Trigger-derived operation.
    pub operation: String,
    /// Authored actor; not authenticated identity.
    pub actor: String,
    /// Authored reason.
    pub reason: String,
    /// CAS/unconditional label.
    pub guard: String,
    /// Database session login, independent of authored actor.
    pub recorded_by: String,
    /// Full previous row image, if present.
    pub before: Option<StoredState>,
    /// Full resulting row image.
    pub after: StoredState,
}
impl std::fmt::Debug for HistoryEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HistoryEntry")
            .field("version", &self.version)
            .finish_non_exhaustive()
    }
}
impl PooledStore {
    /// Read at most 1000 audit rows after `after`, in commit order. `key = None`
    /// reads the whole stream; pagination never uses offset or sequence allocation.
    pub async fn history(
        &self,
        key: Option<&FlagKey>,
        after: Version,
        deadline: Instant,
    ) -> Result<Vec<HistoryEntry>, StoreError> {
        if after.get() > i64::MAX as u64 {
            return Ok(Vec::new());
        }
        let deadline = deadline.min(Instant::now() + self.timing.acquire);
        bounded(deadline, async {
            PgReadOnlySnapshot::inspect(&self.pool, async |sql| {
                let [statement, lock, idle] = self.timing.settings(deadline)?;
                sqlx::query("SELECT set_config('statement_timeout',$1,true), set_config('lock_timeout',$2,true), set_config('idle_in_transaction_session_timeout',$3,true)")
                    .bind(statement).bind(lock).bind(idle).execute(sql.executor()).await?;
                let rows = sqlx::query("SELECT version, flag_name, operation, actor, reason, guard, recorded_by::text, before_state, after_state FROM flags.flag_audit WHERE version > $1 AND ($2::text IS NULL OR flag_name = $2) ORDER BY version LIMIT 1000")
                    .bind(after.get() as i64).bind(key.map(FlagKey::as_str)).fetch_all(sql.executor()).await?;
                rows.into_iter().map(|r| Ok(HistoryEntry {
                    version: crate::load::version(r.try_get("version")?)?, key: r.try_get("flag_name")?,
                    operation: r.try_get("operation")?, actor: r.try_get("actor")?, reason: r.try_get("reason")?,
                    guard: r.try_get("guard")?, recorded_by: r.try_get("recorded_by")?,
                    before: r.try_get::<Option<serde_json::Value>, _>("before_state")?.map(StoredState),
                    after: StoredState(r.try_get("after_state")?),
                })).collect::<Result<Vec<_>, StoreError>>()
            }).await.map_err(|e| StoreError(ErrorKind::Snapshot(Box::new(e))))
        }).await
    }
}
