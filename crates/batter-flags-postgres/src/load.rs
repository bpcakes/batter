use crate::{
    ErrorClass, PooledStore, REQUIRED_SCHEMA_REVISION, StoreError,
    errors::{ErrorKind, check_primary},
    store::bounded,
};
use batter_flags_core::{
    Version,
    low_level::{Candidate, CandidateRow, ValidationScope},
};
use batter_sqlx::PgReadOnlySnapshot;
use sqlx::Row;
use tokio::time::Instant;

/// Coherent primary snapshot, before catalog validation. Archived keys are absent.
/// Its version cannot be relabelled before validation:
/// ```compile_fail,E0616
/// fn relabel(mut loaded: batter_flags_postgres::Loaded) {
///     loaded.version = batter_flags_core::Version::new(100);
/// }
/// ```
#[derive(Debug)]
pub struct Loaded {
    /// Counter read in the same repeatable-read transaction as the rows.
    pub(crate) version: Version,
    /// Installed schema revision.
    pub(crate) schema_revision: i32,
    /// Stored bounded notification channel; payloads contain only a counter.
    pub(crate) channel: String,
    rows: Vec<CandidateRow>,
}
impl Loaded {
    /// Counter read with these rows; cannot be reassigned before candidate conversion.
    pub fn version(&self) -> Version {
        self.version
    }
    /// Installed schema revision.
    pub fn schema_revision(&self) -> i32 {
        self.schema_revision
    }
    /// Bounded schema-derived notification channel.
    pub fn channel(&self) -> &str {
        &self.channel
    }

    /// Attach this database read to the consumer's validation scope.
    pub fn candidate(self, scope: &ValidationScope) -> Candidate {
        Candidate::new(scope).at(self.version).rows(self.rows)
    }
    /// Raw candidate rows for persistence or inspection; formatting is redacted.
    pub fn rows(&self) -> &[CandidateRow] {
        &self.rows
    }
}
impl PooledStore {
    /// Load one REPEATABLE READ READ ONLY primary snapshot. A standby returns
    /// `ErrorClass::Standby` and no candidate, so it cannot renew freshness.
    /// The whole inspection is additionally capped by the acquisition budget,
    /// because the native snapshot owner does not expose a separate acquire phase.
    pub async fn load(&self, deadline: Instant) -> Result<Loaded, StoreError> {
        let deadline = deadline.min(Instant::now() + self.timing.acquire);
        bounded(deadline, async {
            PgReadOnlySnapshot::inspect(&self.pool, async |sql| {
                let [statement, lock, idle] = self.timing.settings(deadline)?;
                sqlx::query("SELECT set_config('statement_timeout',$1,true), set_config('lock_timeout',$2,true), set_config('idle_in_transaction_session_timeout',$3,true)")
                    .bind(statement).bind(lock).bind(idle).execute(sql.executor()).await?;
                let recovery: bool = sqlx::query_scalar("SELECT pg_is_in_recovery()")
                    .fetch_one(sql.executor()).await?;
                check_primary(recovery)?;
                let row = sqlx::query("SELECT version, schema_revision, channel FROM flags.flag_control WHERE singleton")
                    .fetch_one(sql.executor()).await?;
                let version = version(row.try_get("version")?)?;
                let schema_revision: i32 = row.try_get("schema_revision")?;
                if schema_revision < REQUIRED_SCHEMA_REVISION { return Err(StoreError::classified(ErrorClass::SchemaRevision)); }
                let rows = sqlx::query("SELECT name, kind, value_type, updated_version, state::text FROM flags.flag_state WHERE archived_version IS NULL ORDER BY name")
                    .fetch_all(sql.executor()).await?.into_iter().map(|r| {
                        Ok(CandidateRow::new(r.try_get("name")?, r.try_get("kind")?, r.try_get("value_type")?,
                            crate::load::version(r.try_get("updated_version")?)?, r.try_get("state")?))
                    }).collect::<Result<Vec<_>, StoreError>>()?;
                Ok(Loaded { version, schema_revision, channel: row.try_get("channel")?, rows })
            }).await.map_err(|e| match e {
                batter_sqlx::PgSnapshotError::Inspection(e) => e,
                e => StoreError(ErrorKind::Snapshot(Box::new(e))),
            })
        }).await
    }
}
pub(crate) fn version(value: i64) -> Result<Version, StoreError> {
    u64::try_from(value)
        .map(Version::new)
        .map_err(|_| StoreError::classified(ErrorClass::InvalidData))
}
