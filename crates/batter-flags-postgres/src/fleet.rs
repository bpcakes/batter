//! The fleet view: every process row read in one snapshot with the store's
//! head, classified on the server's clock.
use crate::{
    ErrorClass, KeySample, PooledStore, ProcessId, SnapshotReport, StoreError,
    errors::check_primary,
    load::{snapshot_error, version},
};
use batter_flags_core::{FlagKey, Version};
use batter_sqlx::PgReadOnlySnapshot;
use chrono::{DateTime, Utc};
use sqlx::{Row, postgres::PgRow};
use std::num::NonZeroUsize;
use tokio::time::Instant;

/// A process row's state on the server clock when the fleet was read.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ProcessState {
    /// Not stopped, and its heartbeat has not expired.
    Live,
    /// Not stopped, but its heartbeat expired; kept until prune removes it.
    Silent,
    /// The process recorded its stop.
    Stopped,
}

/// One process row.
#[derive(Clone, Debug)]
pub struct FleetProcess {
    process: ProcessId,
    application: String,
    environment: String,
    build: String,
    state: ProcessState,
    snapshot: SnapshotReport,
    declared_revision: [u8; 32],
    held: KeySample,
    floor_engaged: KeySample,
    last_seen_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
}

impl FleetProcess {
    /// The process id.
    pub fn process(&self) -> ProcessId {
        self.process
    }
    /// The application name.
    pub fn application(&self) -> &str {
        &self.application
    }
    /// The environment class.
    pub fn environment(&self) -> &str {
        &self.environment
    }
    /// The build.
    pub fn build(&self) -> &str {
        &self.build
    }
    /// The process's state when the fleet was read.
    pub fn state(&self) -> ProcessState {
        self.state
    }
    /// What the process last reported publishing.
    pub fn snapshot(&self) -> SnapshotReport {
        self.snapshot
    }
    /// The revision the process resolved its declared base to.
    pub fn declared_revision(&self) -> &[u8; 32] {
        &self.declared_revision
    }
    /// Keys the process holds.
    pub fn held(&self) -> &KeySample {
        &self.held
    }
    /// Kill switches its settings floor holds engaged.
    pub fn floor_engaged(&self) -> &KeySample {
        &self.floor_engaged
    }
    /// The server time of its last heartbeat.
    pub fn last_seen_at(&self) -> DateTime<Utc> {
        self.last_seen_at
    }
    /// The server time its row stops being Live.
    pub fn expires_at(&self) -> DateTime<Utc> {
        self.expires_at
    }

    fn read(row: &PgRow) -> Result<Self, StoreError> {
        let invalid = || StoreError::classified(ErrorClass::InvalidData);
        let state = match row.try_get::<&str, _>("state")? {
            "live" => ProcessState::Live,
            "silent" => ProcessState::Silent,
            "stopped" => ProcessState::Stopped,
            _ => return Err(invalid()),
        };
        Ok(Self {
            process: ProcessId::stored(row.try_get("process_id")?),
            application: row.try_get("application")?,
            environment: row.try_get("environment")?,
            build: row.try_get("build")?,
            state,
            snapshot: SnapshotReport::stored(
                row.try_get("snapshot_source")?,
                row.try_get("snapshot_version")?,
            )?,
            declared_revision: row
                .try_get::<Vec<u8>, _>("declared_revision")?
                .try_into()
                .map_err(|_| invalid())?,
            held: KeySample::stored(row.try_get("held_keys")?, row.try_get("held_overflow")?)?,
            floor_engaged: KeySample::stored(
                row.try_get("floor_engaged_keys")?,
                row.try_get("floor_overflow")?,
            )?,
            last_seen_at: row.try_get("last_seen_at")?,
            expires_at: row.try_get("expires_at")?,
        })
    }
}

/// Every process row and the store's head, read together.
#[derive(Clone, Debug)]
pub struct Fleet {
    head: Version,
    processes: Vec<FleetProcess>,
}

/// Whether a version reached every Live process.
///
/// Only a control-plane publication at or above the version counts: a process
/// serving a persisted file or its declared base has not applied it. Silent
/// and Stopped processes are not counted either way.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Rollout {
    /// Every Live process applied the version.
    Reached {
        /// How many Live processes there are.
        live: NonZeroUsize,
    },
    /// At least one Live process has not applied it.
    Pending {
        /// Live processes that applied it.
        reached: usize,
        /// Live processes that did not, in build then process order.
        behind: Vec<ProcessId>,
    },
    /// No process is Live, so nothing can be said to have applied it.
    NoLiveProcess,
}

impl Fleet {
    /// The store's head when the fleet was read.
    pub fn head(&self) -> Version {
        self.head
    }

    /// Every process row, in build then process order.
    pub fn processes(&self) -> &[FleetProcess] {
        &self.processes
    }

    /// Answers "did `version` reach every Live process?".
    pub fn rollout(&self, version: Version) -> Rollout {
        let live = self
            .processes
            .iter()
            .filter(|process| process.state == ProcessState::Live);
        let (reached, behind): (Vec<_>, Vec<_>) = live.partition(|process| {
            matches!(process.snapshot, SnapshotReport::ControlPlane(applied) if applied >= version)
        });
        match (NonZeroUsize::new(reached.len()), behind.is_empty()) {
            (Some(live), true) => Rollout::Reached { live },
            (None, true) => Rollout::NoLiveProcess,
            (_, false) => Rollout::Pending {
                reached: reached.len(),
                behind: behind.iter().map(|process| process.process).collect(),
            },
        }
    }
}

impl PooledStore {
    /// Reads every process row with the head in one REPEATABLE READ READ
    /// ONLY transaction on the primary; a standby is
    /// [`ErrorClass::Standby`](crate::ErrorClass::Standby), because its rows
    /// can lag the primary's.
    pub async fn fleet(&self, deadline: Instant) -> Result<Fleet, StoreError> {
        self.read_fleet(None, deadline).await
    }

    /// Reads the process rows whose catalog declares `key`, with the head.
    pub async fn fleet_declaring(
        &self,
        key: &FlagKey,
        deadline: Instant,
    ) -> Result<Fleet, StoreError> {
        self.read_fleet(Some(key.as_str()), deadline).await
    }

    async fn read_fleet(&self, key: Option<&str>, deadline: Instant) -> Result<Fleet, StoreError> {
        let deadline = deadline.min(Instant::now() + self.timing.acquire);
        PgReadOnlySnapshot::inspect_until(&self.pool, deadline, async |sql| {
            let [statement, lock, idle] = self.timing.settings(deadline)?;
            sqlx::query("SELECT set_config('statement_timeout',$1,true), set_config('lock_timeout',$2,true), set_config('idle_in_transaction_session_timeout',$3,true)")
                .bind(statement).bind(lock).bind(idle).execute(sql.executor()).await?;
            // A standby can lag: a process still heartbeating on the primary
            // may read as expired or be missing, which would let a pending
            // rollout read as reached.
            check_primary(sqlx::query_scalar("SELECT pg_is_in_recovery()").fetch_one(sql.executor()).await?)?;
            let head = version(sqlx::query_scalar("SELECT version FROM flags.flag_control WHERE singleton").fetch_one(sql.executor()).await?)?;
            let processes = sqlx::query("SELECT p.process_id, p.application, p.environment, p.build, \
                CASE WHEN p.stopped_at IS NOT NULL THEN 'stopped' WHEN p.expires_at > clock_timestamp() THEN 'live' ELSE 'silent' END AS state, \
                p.snapshot_source, p.snapshot_version, p.declared_revision, p.held_keys, p.held_overflow, p.floor_engaged_keys, p.floor_overflow, p.last_seen_at, p.expires_at \
                FROM flags.flag_processes p \
                WHERE $1::text IS NULL OR EXISTS (SELECT FROM flags.flag_catalog_entries e WHERE e.manifest_digest = p.manifest_digest AND e.key = $1) \
                ORDER BY p.build, p.process_id")
                .bind(key).fetch_all(sql.executor()).await?
                .iter().map(FleetProcess::read).collect::<Result<Vec<_>, StoreError>>()?;
            Ok(Fleet { head, processes })
        })
        .await
        .map_err(snapshot_error)
    }
}
