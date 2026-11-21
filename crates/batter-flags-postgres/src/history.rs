use crate::{PooledStore, StoreError, StoredState};
use batter_flags_core::{FlagKey, Version};
use batter_sqlx::PgReadOnlySnapshot;
use sqlx::{Row, postgres::PgRow};
use tokio::time::Instant;

/// An audit read: the columns every history read selects, then `$tail`.
macro_rules! audit {
    ($tail:literal) => {
        concat!(
            "SELECT version, publication_id, flag_name, operation, actor, reason, guard, recorded_by::text, \
             recorded_at, before_state, after_state FROM flags.flag_audit ",
            $tail
        )
    };
}

/// One retained audit event. Actor, reason and state are deliberately explicit fields.
/// Default formatting discloses only the counter.
pub struct HistoryEntry {
    /// The immutable write identified by this event.
    /// Migration: read the former `HistoryEntry::version` field with `write_id.version()`.
    pub write_id: crate::WriteId,
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

/// The newest retained audit events of one key, and whether older ones
/// precede them.
#[derive(Debug)]
pub struct RecentHistory {
    /// At most [`HISTORY_PAGE`] events, in commit order, the newest last.
    pub entries: Vec<HistoryEntry>,
    /// Whether older events precede the first entry.
    pub earlier: Earlier,
}

/// Whether a recent history read reached the start of the key's stream.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Earlier {
    /// No retained event precedes the first entry.
    None,
    /// Older events are retained: [`PooledStore::history`] reads them from
    /// the start, page by page.
    Retained,
}

impl HistoryEntry {
    fn read(r: &PgRow) -> Result<Self, StoreError> {
        let version = crate::load::version(r.try_get("version")?)?;
        Ok(Self {
            write_id: crate::WriteId {
                version,
                event: r.try_get("publication_id")?,
            },
            key: r.try_get("flag_name")?,
            operation: r.try_get("operation")?,
            actor: r.try_get("actor")?,
            reason: r.try_get("reason")?,
            guard: r.try_get("guard")?,
            recorded_by: r.try_get("recorded_by")?,
            recorded_at: r.try_get("recorded_at")?,
            before: r
                .try_get::<Option<serde_json::Value>, _>("before_state")?
                .map(StoredState),
            after: StoredState(r.try_get("after_state")?),
        })
    }
}

impl std::fmt::Debug for HistoryEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HistoryEntry")
            .field("version", &self.write_id.version())
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
    /// page, but only after the same guard: a standby or an unusable schema is
    /// refused whatever the cursor.
    pub async fn history(
        &self,
        key: Option<&FlagKey>,
        after: Version,
        deadline: Instant,
    ) -> Result<HistoryPage, StoreError> {
        let cursor = i64::try_from(after.get());
        let deadline = deadline.min(Instant::now() + self.timing.acquire);
        PgReadOnlySnapshot::inspect_until(&self.pool, deadline, async |sql| {
            self.read_guard(sql, deadline).await?;
            let Ok(after) = cursor else {
                return Ok(HistoryPage {
                    entries: Vec::new(),
                    next: Continuation::Complete,
                });
            };
            // One row beyond the page says whether more follow. A keyed
            // read has its own statement, so it walks the keyed index.
            let limit = HISTORY_PAGE as i64 + 1;
            let rows = match key {
                None => {
                    sqlx::query(audit!("WHERE version > $1 ORDER BY version LIMIT $2"))
                        .bind(after)
                        .bind(limit)
                        .fetch_all(sql.executor())
                        .await?
                }
                Some(key) => {
                    sqlx::query(audit!(
                        "WHERE flag_name = $1 AND version > $2 ORDER BY version LIMIT $3"
                    ))
                    .bind(key.as_str())
                    .bind(after)
                    .bind(limit)
                    .fetch_all(sql.executor())
                    .await?
                }
            };
            let more = rows.len() > HISTORY_PAGE;
            let entries = rows
                .iter()
                .take(HISTORY_PAGE)
                .map(HistoryEntry::read)
                .collect::<Result<Vec<_>, StoreError>>()?;
            let next = match (more, entries.last()) {
                (true, Some(last)) => Continuation::After(last.write_id.version()),
                (false, _) | (true, None) => Continuation::Complete,
            };
            Ok(HistoryPage { entries, next })
        })
        .await
        .map_err(crate::load::snapshot_error)
    }

    /// Reads the newest [`HISTORY_PAGE`] audit events of `key`, in commit
    /// order, and says whether older ones precede them: the events an
    /// operator reconciling an uncertain write needs, in one bounded read
    /// however long the key's history is. The read refuses a standby and a
    /// schema this build cannot read, in its own snapshot, as
    /// [`history`](Self::history) does.
    ///
    /// ```no_run
    /// use batter_flags_core::FlagKey;
    /// use batter_flags_postgres::{Earlier, PooledStore};
    /// use std::time::Duration;
    /// use tokio::time::Instant;
    /// # async fn example(store: PooledStore) -> Result<(), Box<dyn std::error::Error>> {
    /// let recent = store
    ///     .recent_history(&FlagKey::new("outbound")?, Instant::now() + Duration::from_secs(3))
    ///     .await?;
    /// if let Some(newest) = recent.entries.last() {
    ///     println!("newest change: version {}", newest.write_id.version().get());
    /// }
    /// if recent.earlier == Earlier::Retained {
    ///     // Older events: page forward from the start with `history`.
    /// }
    /// # Ok(()) }
    /// ```
    pub async fn recent_history(
        &self,
        key: &FlagKey,
        deadline: Instant,
    ) -> Result<RecentHistory, StoreError> {
        let deadline = deadline.min(Instant::now() + self.timing.acquire);
        PgReadOnlySnapshot::inspect_until(&self.pool, deadline, async |sql| {
            self.read_guard(sql, deadline).await?;
            // Newest first, one row beyond the page saying whether older
            // events follow, then back into commit order.
            let rows = sqlx::query(audit!(
                "WHERE flag_name = $1 ORDER BY version DESC LIMIT $2"
            ))
            .bind(key.as_str())
            .bind(HISTORY_PAGE as i64 + 1)
            .fetch_all(sql.executor())
            .await?;
            let earlier = if rows.len() > HISTORY_PAGE {
                Earlier::Retained
            } else {
                Earlier::None
            };
            let mut entries = rows
                .iter()
                .take(HISTORY_PAGE)
                .map(HistoryEntry::read)
                .collect::<Result<Vec<_>, StoreError>>()?;
            entries.reverse();
            Ok(RecentHistory { entries, earlier })
        })
        .await
        .map_err(crate::load::snapshot_error)
    }
}
