//! The fleet view: every process row read in one snapshot with the store's
//! head, classified on the server's clock.
use crate::{
    ErrorClass, KeySample, PooledStore, ProcessId, SnapshotReport, StoreError,
    load::{snapshot_error, version},
};
use batter_flags_core::{FlagKey, Version};
use batter_sqlx::PgReadOnlySnapshot;
use chrono::{DateTime, Utc};
use sqlx::{Row, postgres::PgRow};
use std::num::NonZeroUsize;
use tokio::time::Instant;

/// A process row's state on the server clock when the fleet was read. Every
/// row of one [`Fleet`] is classified at the same server instant, the start of
/// the read's transaction, which precedes the snapshot the rows come from: a
/// row the snapshot shows expired had expired before the snapshot was taken,
/// so a heartbeat renewed after it cannot make a Live process read Silent.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ProcessState {
    /// Not stopped, and its heartbeat has not expired.
    Live,
    /// Not stopped, but its heartbeat expired; kept until prune removes it.
    Silent,
    /// The process recorded its stop.
    Stopped,
}

#[derive(Clone, Copy, Debug)]
enum Applied {
    Unconfirmed(SnapshotReport),
    Confirmed(Version),
}

/// One process row.
#[derive(Clone, Debug)]
pub struct FleetProcess {
    process: ProcessId,
    application: String,
    environment: String,
    build: String,
    state: ProcessState,
    snapshot: Applied,
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
        match self.snapshot {
            Applied::Unconfirmed(report) => report,
            Applied::Confirmed(version) => SnapshotReport::ControlPlane(version),
        }
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
        let report = SnapshotReport::stored(
            row.try_get("snapshot_source")?,
            row.try_get("snapshot_version")?,
        )?;
        let snapshot = match report {
            SnapshotReport::ControlPlane(version) if row.try_get("confirmed")? => {
                Applied::Confirmed(version)
            }
            report => Applied::Unconfirmed(report),
        };
        Ok(Self {
            process: ProcessId::stored(row.try_get("process_id")?),
            application: row.try_get("application")?,
            environment: row.try_get("environment")?,
            build: row.try_get("build")?,
            state,
            snapshot,
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
/// Only a control-plane publication at or above the version, and no higher
/// than the head read with it, counts: a process serving a persisted file or
/// its declared base has not applied it, and one reporting a version above the
/// head applied a store this one is not, such as the store before a restore.
/// For an in-range report, the publication identity must also match the
/// immutable event in this history. Legacy version-only reports remain pending.
/// The writer ignores a report beyond the jump window, and that process then
/// refuses the write as a regression, so it is listed as behind. Silent and
/// Stopped processes are not counted either way.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Rollout {
    /// Every Live process applied the version.
    Reached {
        /// How many Live processes there are.
        live: NonZeroUsize,
    },
    /// At least one Live process lacks confirmation for this version.
    Pending {
        /// Live processes that applied it.
        reached: usize,
        /// Live processes without confirmation in this history, in build then process order.
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
            matches!(process.snapshot, Applied::Confirmed(applied)
                if applied >= version && applied <= self.head)
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
            // A standby can lag: a process still heartbeating on the primary
            // may read as expired or be missing, which would let a pending
            // rollout read as reached. The guard refuses it, and a schema
            // this build cannot read.
            self.read_guard(sql, deadline).await?;
            // The guard's first statement took the snapshot; the cutoff below
            // is the transaction's start, before it, so every row is judged
            // at an instant no later than the state it shows.
            let head = version(sqlx::query_scalar("SELECT version FROM flags.flag_control WHERE singleton").fetch_one(sql.executor()).await?)?;
            let processes = sqlx::query("SELECT p.process_id, p.application, p.environment, p.build, \
                CASE WHEN p.stopped_at IS NOT NULL THEN 'stopped' WHEN p.expires_at > transaction_timestamp() THEN 'live' ELSE 'silent' END AS state, \
                p.snapshot_source, p.snapshot_version, \
                CASE WHEN p.snapshot_version = 0 THEN p.publication_id IS NOT DISTINCT FROM '00000000-0000-0000-0000-000000000000'::uuid \
                     ELSE EXISTS (SELECT FROM flags.flag_audit a WHERE a.version = p.snapshot_version AND a.publication_id = p.publication_id) END AS confirmed, \
                p.declared_revision, p.held_keys, p.held_overflow, p.floor_engaged_keys, p.floor_overflow, p.last_seen_at, p.expires_at \
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
