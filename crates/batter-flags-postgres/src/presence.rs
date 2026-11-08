//! Process presence: the heartbeat that publishes a catalog and refreshes a
//! process row, and the marker that records a process stopping.
//!
//! Process states are judged on the server's clock. A process is Live while
//! it has not stopped and its row has not expired, Silent once its row has
//! expired without a stop, and Stopped once it recorded its stop.
use crate::{ErrorClass, PooledStore, StoreError, ValidationError, catalogs::CatalogPublication};
use batter_flags_core::{FlagKey, Version, low_level::Base};
use batter_sqlx::PgScopedSql;
use std::time::Duration;
use tokio::time::Instant;
use uuid::Uuid;

mod stop;
pub use stop::{StopFailure, StopOutcome};

/// The highest state encoding this build decodes, reported on every heartbeat.
const ROW_ENCODING: i16 = 1;
/// The most keys a heartbeat lists in one sample; the rest are counted.
const SAMPLE_LIMIT: usize = 64;

/// A process's identity in the store, a UUID v7 minted once per process.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ProcessId(Uuid);

impl ProcessId {
    pub(crate) fn stored(uuid: Uuid) -> Self {
        Self(uuid)
    }

    /// The UUID as stored in `flag_processes.process_id`.
    pub fn as_uuid(&self) -> Uuid {
        self.0
    }
}

/// An application, environment or build name on a process row: 1 to 96
/// ASCII letters, digits, `.`, `_`, `-` or `+`.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ProcessLabel(String);

impl ProcessLabel {
    /// Validates `value`.
    pub fn new(value: &str) -> Result<Self, ValidationError> {
        let allowed = |byte: u8| byte.is_ascii_alphanumeric() || b"._-+".contains(&byte);
        if value.is_empty() || value.len() > 96 || !value.bytes().all(allowed) {
            return Err(ValidationError);
        }
        Ok(Self(value.to_owned()))
    }

    /// The validated text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// What a process has published, as its heartbeat reports it.
///
/// Only [`ControlPlane`](Self::ControlPlane) counts as having applied a
/// version: a persisted file or the declared base never blocks a write and
/// never counts toward a rollout.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SnapshotReport {
    /// Nothing is published yet; the process serves its declared base.
    NotLoaded,
    /// The last-good file is published at this version.
    PersistedFile(Version),
    /// A control-plane read is published at this version.
    ControlPlane(Version),
}

impl SnapshotReport {
    fn columns(self) -> Result<(&'static str, Option<i64>), StoreError> {
        let stored = |version: Version| {
            i64::try_from(version.get())
                .map_err(|_| StoreError::classified(ErrorClass::InvalidData))
        };
        Ok(match self {
            Self::NotLoaded => ("not_loaded", None),
            Self::PersistedFile(version) => ("persisted_file", Some(stored(version)?)),
            Self::ControlPlane(version) => ("control_plane", Some(stored(version)?)),
        })
    }

    pub(crate) fn stored(source: &str, stored: Option<i64>) -> Result<Self, StoreError> {
        let version = |version: Option<i64>| {
            version
                .ok_or_else(|| StoreError::classified(ErrorClass::InvalidData))
                .and_then(crate::load::version)
        };
        match (source, stored) {
            ("not_loaded", None) => Ok(Self::NotLoaded),
            ("persisted_file", v) => Ok(Self::PersistedFile(version(v)?)),
            ("control_plane", v) => Ok(Self::ControlPlane(version(v)?)),
            _ => Err(StoreError::classified(ErrorClass::InvalidData)),
        }
    }
}

/// A bounded list of keys: at most 64, sorted, with a count of the rest.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct KeySample {
    keys: Vec<FlagKey>,
    overflow: u64,
}

impl KeySample {
    /// Samples `keys`: duplicates are dropped, the first 64 in key order are
    /// listed and the others counted.
    pub fn of(keys: impl IntoIterator<Item = FlagKey>) -> Self {
        let mut keys: Vec<FlagKey> = keys.into_iter().collect();
        keys.sort_unstable_by(|a, b| a.as_str().cmp(b.as_str()));
        keys.dedup_by(|a, b| a.as_str() == b.as_str());
        let rest = keys.split_off(keys.len().min(SAMPLE_LIMIT));
        Self {
            keys,
            overflow: rest.len() as u64,
        }
    }

    /// The listed keys, in key order.
    pub fn keys(&self) -> &[FlagKey] {
        &self.keys
    }

    /// How many keys were not listed.
    pub fn overflow(&self) -> u64 {
        self.overflow
    }

    fn columns(&self) -> Result<(Vec<&str>, i64), StoreError> {
        let overflow = i64::try_from(self.overflow)
            .map_err(|_| StoreError::classified(ErrorClass::InvalidData))?;
        Ok((self.keys.iter().map(FlagKey::as_str).collect(), overflow))
    }

    pub(crate) fn stored(keys: Vec<String>, overflow: i64) -> Result<Self, StoreError> {
        let invalid = |_| StoreError::classified(ErrorClass::InvalidData);
        Ok(Self {
            keys: keys
                .iter()
                .map(|key| {
                    FlagKey::new(key).map_err(|_| StoreError::classified(ErrorClass::InvalidData))
                })
                .collect::<Result<_, _>>()?,
            overflow: u64::try_from(overflow).map_err(invalid)?,
        })
    }
}

/// How long a process row stays Live after a heartbeat, and how long it is
/// kept after expiring. Both are whole seconds from 1 s to 24 h, and each row
/// carries its own, so processes with different cadences never prune each
/// other early.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Cadence {
    liveness: i32,
    retention: i32,
}

impl Cadence {
    /// Validates both durations.
    pub fn new(liveness: Duration, retention: Duration) -> Result<Self, ValidationError> {
        let seconds = |duration: Duration| {
            let whole = duration.subsec_nanos() == 0;
            match i32::try_from(duration.as_secs()) {
                Ok(secs @ 1..=86_400) if whole => Ok(secs),
                _ => Err(ValidationError),
            }
        };
        Ok(Self {
            liveness: seconds(liveness)?,
            retention: seconds(retention)?,
        })
    }
}

/// Everything about a process that does not change between heartbeats, and
/// the capability to send them.
///
/// Its process id is minted here, and its catalog and both revisions come
/// from one [`Base`], so a process cannot share another's row or report a
/// catalog it did not resolve. It is not `Clone`: heartbeats take it by
/// `&mut`, so one process sends one heartbeat at a time, and
/// [`PooledStore::stop`] consumes it, so nothing can heartbeat a stopped
/// process back to life.
///
/// ```compile_fail,E0382
/// # async fn example(store: batter_flags_postgres::PooledStore,
/// #     mut presence: batter_flags_postgres::ProcessPresence,
/// #     report: batter_flags_postgres::PresenceReport) {
/// # let deadline = tokio::time::Instant::now();
/// let _ = store.stop(presence, deadline).await;
/// let _ = store.heartbeat(&mut presence, &report, deadline).await;
/// # }
/// ```
#[derive(Debug)]
pub struct ProcessPresence {
    process: ProcessId,
    application: ProcessLabel,
    environment: ProcessLabel,
    build: ProcessLabel,
    catalog: CatalogPublication,
    revision: [u8; 32],
    shared_revision: [u8; 32],
    cadence: Cadence,
}

/// The names a process row carries.
#[derive(Clone, Debug)]
pub struct ProcessLabels {
    /// The application the process belongs to.
    pub application: ProcessLabel,
    /// The environment class the process resolved for.
    pub environment: ProcessLabel,
    /// The build the process runs.
    pub build: ProcessLabel,
}

impl ProcessPresence {
    /// Mints a process id for the process whose declared base is `base`:
    /// its catalog's manifest and both of its revisions are taken from it.
    pub fn new(labels: ProcessLabels, base: &Base, cadence: Cadence) -> Self {
        Self {
            process: ProcessId(Uuid::now_v7()),
            application: labels.application,
            environment: labels.environment,
            build: labels.build,
            catalog: CatalogPublication::new(&base.catalog().manifest()),
            revision: *base.revision().as_bytes(),
            shared_revision: *base.shared_revision().as_bytes(),
            cadence,
        }
    }

    /// The minted process id.
    pub fn process(&self) -> ProcessId {
        self.process
    }
}

/// What changes between heartbeats.
#[derive(Clone, Debug)]
pub struct PresenceReport {
    /// What the process has published.
    pub snapshot: SnapshotReport,
    /// Keys the process holds because it could not apply their rows.
    pub held: KeySample,
    /// Kill switches the process's settings floor holds engaged.
    pub floor_engaged: KeySample,
}

impl PooledStore {
    /// Sends one heartbeat: publishes the catalog if the store does not keep
    /// it, refreshes its reference at most once a day, and upserts the
    /// process row. Its expiry is stamped from the server clock when the row
    /// is written, after any wait for the row's lock, so a slow heartbeat
    /// never commits an already expired row. A heartbeat never clears a
    /// recorded stop. It is not audited and not retried; the next heartbeat
    /// restates presence, so [`ErrorClass::CommitUnknown`] needs no repair.
    pub async fn heartbeat(
        &self,
        presence: &mut ProcessPresence,
        report: &PresenceReport,
        deadline: Instant,
    ) -> Result<(), StoreError> {
        let (source, version) = report.snapshot.columns()?;
        let (held, held_overflow) = report.held.columns()?;
        let (floor, floor_overflow) = report.floor_engaged.columns()?;
        self.atomically(deadline, async |sql| {
            let digest = &presence.catalog.manifest_digest()[..];
            // A concurrent prune may delete the catalog between an insert that
            // found it and the read that locks it; publishing again settles it.
            let mut attempts = 0;
            while !referenced(sql, digest).await? {
                attempts += 1;
                if attempts > 3 {
                    return Err(StoreError::classified(ErrorClass::Database));
                }
                presence.catalog.insert(sql).await?;
            }
            sqlx::query("UPDATE flags.flag_catalogs SET last_referenced_at = clock_timestamp() WHERE manifest_digest = $1 AND last_referenced_at < clock_timestamp() - interval '1 day'")
                .bind(digest).execute(sql.executor()).await?;
            sqlx::query("INSERT INTO flags.flag_processes(process_id, application, environment, build, manifest_digest, snapshot_source, snapshot_version, declared_revision, declared_shared_revision, row_encoding, held_keys, held_overflow, floor_engaged_keys, floor_overflow, liveness_seconds, retention_seconds, expires_at, last_seen_at) \
                VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16, clock_timestamp() + $15 * interval '1 second', clock_timestamp()) \
                ON CONFLICT (process_id) DO UPDATE SET snapshot_source = EXCLUDED.snapshot_source, snapshot_version = EXCLUDED.snapshot_version, row_encoding = EXCLUDED.row_encoding, held_keys = EXCLUDED.held_keys, held_overflow = EXCLUDED.held_overflow, floor_engaged_keys = EXCLUDED.floor_engaged_keys, floor_overflow = EXCLUDED.floor_overflow, liveness_seconds = EXCLUDED.liveness_seconds, retention_seconds = EXCLUDED.retention_seconds, expires_at = clock_timestamp() + EXCLUDED.liveness_seconds * interval '1 second', last_seen_at = clock_timestamp()")
                .bind(presence.process.0)
                .bind(presence.application.as_str())
                .bind(presence.environment.as_str())
                .bind(presence.build.as_str())
                .bind(digest)
                .bind(source)
                .bind(version)
                .bind(&presence.revision[..])
                .bind(&presence.shared_revision[..])
                .bind(ROW_ENCODING)
                .bind(&held)
                .bind(held_overflow)
                .bind(&floor)
                .bind(floor_overflow)
                .bind(presence.cadence.liveness)
                .bind(presence.cadence.retention)
                .execute(sql.executor()).await?;
            Ok(())
        })
        .await
    }
}

/// Takes a key-share lock on the stored catalog. It conflicts neither with
/// itself nor with the process upsert, so a build's heartbeats never queue on
/// their catalog, and prune's `FOR UPDATE SKIP LOCKED` passes it by.
async fn referenced(sql: &mut PgScopedSql<'_>, digest: &[u8]) -> Result<bool, StoreError> {
    Ok(sqlx::query(
        "SELECT manifest_digest FROM flags.flag_catalogs WHERE manifest_digest = $1 FOR KEY SHARE",
    )
    .bind(digest)
    .fetch_optional(sql.executor())
    .await?
    .is_some())
}
