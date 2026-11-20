use crate::{ErrorClass, REQUIRED_SCHEMA_REVISION, ServingStore, StoreError, errors::ErrorKind};
use batter_flags_core::{
    Version,
    low_level::{Candidate, CandidateRow, ValidationScope},
};
use sqlx::Row;
use tokio::time::Instant;

/// The immutable audit event at a loaded snapshot's head. Only a coherent
/// primary read can produce this value. The initial empty store has no event.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Publication(Event);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Event {
    Initial,
    Write(crate::WriteId),
}

impl Publication {
    /// The counter read with this event.
    pub fn version(self) -> Version {
        match self.0 {
            Event::Initial => Version::new(0),
            Event::Write(write) => write.version(),
        }
    }
    pub(crate) fn id(self) -> uuid::Uuid {
        match self.0 {
            Event::Initial => uuid::Uuid::nil(),
            Event::Write(write) => write.event,
        }
    }
}

/// Coherent primary snapshot, before catalog validation. Archived keys are absent.
/// Its version cannot be relabelled before validation:
/// ```compile_fail,E0616
/// fn relabel(mut loaded: batter_flags_postgres::Loaded) {
///     loaded.publication = todo!();
/// }
/// ```
#[derive(Debug)]
pub struct Loaded {
    /// Counter read in the same repeatable-read transaction as the rows.
    publication: Publication,
    /// Installed schema revision.
    pub(crate) schema_revision: i32,
    rows: Vec<CandidateRow>,
}
impl Loaded {
    /// Counter read with these rows; cannot be reassigned before candidate conversion.
    pub fn version(&self) -> Version {
        self.publication.version()
    }
    /// The event this read observed. Report it only after accepting this read.
    pub fn publication(&self) -> Publication {
        self.publication
    }
    /// Installed schema revision.
    pub fn schema_revision(&self) -> i32 {
        self.schema_revision
    }

    /// Attach this database read to the consumer's validation scope.
    pub fn candidate(self, scope: &ValidationScope) -> Candidate {
        Candidate::new(scope, self.version()).rows(self.rows)
    }
    /// Raw candidate rows for persistence or inspection; formatting is redacted.
    pub fn rows(&self) -> &[CandidateRow] {
        &self.rows
    }
}
impl ServingStore {
    /// Load one REPEATABLE READ READ ONLY primary snapshot. A standby returns
    /// `ErrorClass::Standby` and no candidate, so it cannot renew freshness.
    /// The whole inspection is additionally capped by the acquisition budget,
    /// because the native snapshot owner does not expose a separate acquire phase.
    pub async fn load(&self, deadline: Instant) -> Result<Loaded, StoreError> {
        let deadline = deadline.min(Instant::now() + self.timing.acquire);
        self.inspect(deadline, async |sql| {
            let (head, schema_revision): (i64, i32) = sqlx::query_as(
                "SELECT version, schema_revision FROM flags.flag_control WHERE singleton",
            )
            .fetch_one(sql.executor())
            .await
            .map_err(crate::errors::missing_schema)?;
            if schema_revision < REQUIRED_SCHEMA_REVISION {
                return Err(StoreError::schema_behind(schema_revision));
            }
            let publication = Publication(if head == 0 {
                Event::Initial
            } else {
                let id = sqlx::query_scalar("SELECT publication_id FROM flags.flag_audit WHERE version = $1")
                    .bind(head)
                    .fetch_one(sql.executor()).await?;
                Event::Write(crate::WriteId { version: version(head)?, event: id })
            });
            let rows = sqlx::query("SELECT name, kind, value_type, updated_version, state::text FROM flags.flag_state WHERE archived_version IS NULL ORDER BY name")
                .fetch_all(sql.executor()).await?
                .into_iter().map(|row| {
                    Ok(CandidateRow::new(
                        row.try_get("name")?,
                        row.try_get("kind")?,
                        row.try_get("value_type")?,
                        version(row.try_get("updated_version")?)?,
                        row.try_get("state")?,
                    ))
                }).collect::<Result<Vec<_>, StoreError>>()?;
            Ok(Loaded { publication, schema_revision, rows })
        }).await
    }
}
pub(crate) fn snapshot_error(
    error: batter_sqlx::PgDeadlineError<batter_sqlx::PgSnapshotError<StoreError>, StoreError>,
) -> StoreError {
    match error {
        batter_sqlx::PgDeadlineError::Elapsed => StoreError::classified(ErrorClass::Deadline),
        batter_sqlx::PgDeadlineError::Failed(batter_sqlx::PgSnapshotError::Inspection(error)) => {
            error
        }
        batter_sqlx::PgDeadlineError::Failed(error) => {
            StoreError(ErrorKind::Snapshot(Box::new(error)))
        }
        batter_sqlx::PgDeadlineError::RecoveryElapsed(error) => {
            crate::RollbackFailure::deadline(error)
        }
    }
}
pub(crate) fn version(value: i64) -> Result<Version, StoreError> {
    u64::try_from(value)
        .map(Version::new)
        .map_err(|_| StoreError::classified(ErrorClass::InvalidData))
}
