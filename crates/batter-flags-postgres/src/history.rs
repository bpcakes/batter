use crate::{PooledStore, StoreError, StoredState};
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
    /// When the audit row was written, by the database's clock.
    pub recorded_at: chrono::DateTime<chrono::Utc>,
    /// Full previous row image, if present.
    pub before: Option<StoredState>,
    /// Full resulting row image.
    pub after: StoredState,
}
/// The most audit rows one [`PooledStore::history`] read returns.
pub const HISTORY_PAGE: usize = 1000;

/// One page of retained audit events, and whether more follow it.
#[derive(Debug)]
pub struct HistoryPage {
    /// At most [`HISTORY_PAGE`] events, in commit order.
    pub entries: Vec<HistoryEntry>,
    /// Whether events follow the last entry.
    pub next: Continuation,
}

/// Whether a history read reached the end of the stream.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Continuation {
    /// No retained event follows the last entry.
    Complete,
    /// More follow: read again with `after` set to this version, the last
    /// entry's.
    After(Version),
}

impl std::fmt::Debug for HistoryEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HistoryEntry")
            .field("version", &self.version)
            .finish_non_exhaustive()
    }
}
impl PooledStore {
    /// Reads one page of at most [`HISTORY_PAGE`] audit rows after `after`, in
    /// commit order, and says whether more follow. `key = None` reads the
    /// whole stream; pagination never uses offset or sequence allocation. The
    /// read refuses a standby and a schema this build cannot read, in its own
    /// snapshot, as [`preflight`](Self::preflight) does. No version above the
    /// store's `bigint` range exists, so such an `after` reads a complete, empty
    /// page.
    pub async fn history(
        &self,
        key: Option<&FlagKey>,
        after: Version,
        deadline: Instant,
    ) -> Result<HistoryPage, StoreError> {
        let Ok(after) = i64::try_from(after.get()) else {
            return Ok(HistoryPage {
                entries: Vec::new(),
                next: Continuation::Complete,
            });
        };
        let deadline = deadline.min(Instant::now() + self.timing.acquire);
        PgReadOnlySnapshot::inspect_until(&self.pool, deadline, async |sql| {
                self.read_guard(sql, deadline).await?;
                // One row beyond the page says whether more follow.
                let rows = sqlx::query("SELECT version, flag_name, operation, actor, reason, guard, recorded_by::text, recorded_at, before_state, after_state FROM flags.flag_audit WHERE version > $1 AND ($2::text IS NULL OR flag_name = $2) ORDER BY version LIMIT $3")
                    .bind(after).bind(key.map(FlagKey::as_str)).bind(HISTORY_PAGE as i64 + 1).fetch_all(sql.executor()).await?;
                let more = rows.len() > HISTORY_PAGE;
                let entries = rows.into_iter().take(HISTORY_PAGE).map(|r| Ok(HistoryEntry {
                    version: crate::load::version(r.try_get("version")?)?, key: r.try_get("flag_name")?,
                    operation: r.try_get("operation")?, actor: r.try_get("actor")?, reason: r.try_get("reason")?,
                    guard: r.try_get("guard")?, recorded_by: r.try_get("recorded_by")?,
                    recorded_at: r.try_get("recorded_at")?,
                    before: r.try_get::<Option<serde_json::Value>, _>("before_state")?.map(StoredState),
                    after: StoredState(r.try_get("after_state")?),
                })).collect::<Result<Vec<_>, StoreError>>()?;
                let next = match (more, entries.last()) {
                    (true, Some(last)) => Continuation::After(last.version),
                    (false, _) | (true, None) => Continuation::Complete,
                };
                Ok(HistoryPage { entries, next })
            }).await.map_err(crate::load::snapshot_error)
    }
}
