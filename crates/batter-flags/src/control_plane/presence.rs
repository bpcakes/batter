//! Process presence: the heartbeat, the prune and the stop marker that
//! registration adds, so the fleet view and the archive gate see this
//! process.

use std::{sync::Arc, time::Duration};

use batter_core::{
    BoxError, RegistrationError,
    periodic::{
        PeriodicFailure, PeriodicPolicy, PeriodicReader, PeriodicShutdown, PeriodicStartup,
        register_periodic_in,
    },
    registration::RegistrationTarget,
};
use batter_flags_core::{
    FlagKey, Kind, SwitchState,
    low_level::{Base, ResolutionSource, Standing},
};
use batter_flags_postgres::{
    Cadence, KeySample, PooledStore, PresenceReport, ProcessId, ProcessLabel, ProcessLabels,
    ProcessPresence, PruneLimit, SnapshotReport, StopFailure, StoreError,
};
use tokio::{sync::Mutex, time::Instant};

use super::{
    Budgets, RegisterError,
    status::Shared,
    timing::{MARGIN, work_deadline},
};

/// The shortest heartbeat interval.
const SHORTEST_INTERVAL: Duration = Duration::from_secs(5);
/// The longest heartbeat interval.
const LONGEST_INTERVAL: Duration = Duration::from_secs(300);
/// The longest a stopped or silent row is kept, which bounds how long only
/// silent rows can block a write.
const LONGEST_RETENTION: Duration = Duration::from_secs(86_400);
/// How many rows one prune deletes from each table.
const PRUNE_ROWS: u16 = 1_000;

/// An application name on a process row: 1 to 96 ASCII letters, digits, `.`,
/// `_` or `-`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApplicationName(ProcessLabel);

impl ApplicationName {
    /// Validates `name`.
    pub fn new(name: &str) -> Result<Self, PresenceError> {
        if name.contains('+') {
            return Err(PresenceError::InvalidName);
        }
        ProcessLabel::new(name)
            .map(Self)
            .map_err(|_| PresenceError::InvalidName)
    }
}

/// A build identity on a process row: 1 to 96 ASCII letters, digits, `.`, `_`,
/// `-` or `+`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BuildId(ProcessLabel);

impl BuildId {
    /// Validates `build`.
    pub fn new(build: &str) -> Result<Self, PresenceError> {
        ProcessLabel::new(build)
            .map(Self)
            .map_err(|_| PresenceError::InvalidName)
    }
}

/// How often a process heartbeats and how long its row is kept once it stops
/// or goes silent.
///
/// ```
/// use std::time::Duration;
/// use batter_flags::control_plane::HeartbeatCadence;
///
/// let cadence = HeartbeatCadence::new(Duration::from_secs(15), Duration::from_secs(3_600)).unwrap();
/// // Retention must cover ten intervals.
/// assert!(HeartbeatCadence::new(Duration::from_secs(15), Duration::from_secs(60)).is_err());
/// ```
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HeartbeatCadence {
    interval: Duration,
    retention: Duration,
}

impl HeartbeatCadence {
    /// Validates the cadence: an interval of whole seconds from 5 s to 300 s,
    /// and a retention of whole seconds from ten intervals to 24 h.
    pub fn new(interval: Duration, retention: Duration) -> Result<Self, PresenceError> {
        let whole = |d: Duration| d.subsec_nanos() == 0;
        if !whole(interval) || !(SHORTEST_INTERVAL..=LONGEST_INTERVAL).contains(&interval) {
            return Err(PresenceError::Interval);
        }
        if !whole(retention) || !(interval * 10..=LONGEST_RETENTION).contains(&retention) {
            return Err(PresenceError::Retention);
        }
        Ok(Self {
            interval,
            retention,
        })
    }
}

/// What a process reports about itself to the fleet view.
#[derive(Clone, Debug)]
pub struct Presence {
    application: ApplicationName,
    build: BuildId,
    cadence: HeartbeatCadence,
}

impl Presence {
    /// Presence for one application build.
    pub fn new(application: ApplicationName, build: BuildId, cadence: HeartbeatCadence) -> Self {
        Self {
            application,
            build,
            cadence,
        }
    }
}

/// A presence input outside its range.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum PresenceError {
    /// A name is empty, too long or has a character outside its set.
    #[error("presence names are 1 to 96 ASCII letters, digits, `.`, `_` or `-` (builds also `+`)")]
    InvalidName,
    /// The heartbeat interval is not whole seconds from 5 s to 300 s.
    #[error("the heartbeat interval must be whole seconds from 5 s to 300 s")]
    Interval,
    /// The retention is not whole seconds from ten intervals to 24 h.
    #[error("the presence retention must be whole seconds from ten intervals to 24 h")]
    Retention,
}

/// The presence capability, shared by the heartbeat and the stop marker.
/// Stopping consumes it, so no heartbeat follows the stop.
enum Slot {
    Active(Box<ProcessPresence>),
    Stopped,
}

/// Presence, validated and ready to register.
pub(crate) struct Prepared {
    store: PooledStore,
    slot: Arc<Mutex<Slot>>,
    process: ProcessId,
    floor: KeySample,
    heartbeat: PeriodicPolicy,
    prune: PeriodicPolicy,
    stop_budget: Duration,
}

/// Validates presence for the process whose declared base is `base`.
pub(crate) fn prepare(
    store: &PooledStore,
    presence: Presence,
    base: &Base,
    environment: &str,
    budgets: Budgets,
) -> Result<Prepared, RegisterError> {
    let environment = ProcessLabel::new(environment).map_err(|_| PresenceError::InvalidName)?;
    let liveness = presence.cadence.interval * 2 + whole_seconds(budgets.presence);
    let cadence =
        Cadence::new(liveness, presence.cadence.retention).map_err(|_| PresenceError::Retention)?;
    let heartbeat = PeriodicPolicy::new(
        presence.cadence.interval,
        budgets.presence,
        PeriodicStartup::immediate(),
        PeriodicShutdown::SupportThroughDrain,
    )?;
    let prune = PeriodicPolicy::new(
        presence.cadence.interval * 10,
        budgets.prune,
        PeriodicStartup::immediate(),
        PeriodicShutdown::StopAtDrain,
    )?;
    let labels = ProcessLabels {
        application: presence.application.0,
        environment,
        build: presence.build.0,
    };
    let process_presence = ProcessPresence::new(labels, base, cadence);
    Ok(Prepared {
        store: store.clone(),
        process: process_presence.process(),
        slot: Arc::new(Mutex::new(Slot::Active(Box::new(process_presence)))),
        floor: KeySample::of(floor_engaged(base)),
        heartbeat,
        prune,
        stop_budget: budgets.presence,
    })
}

impl Prepared {
    pub(crate) fn process(&self) -> ProcessId {
        self.process
    }

    /// Reserves the stop marker. It runs before the application's pool
    /// closes, because the pool was registered first and cleanup runs in
    /// reverse.
    pub(crate) fn reserve_stop<T: RegistrationTarget + ?Sized>(
        &self,
        target: &mut T,
    ) -> Result<(), RegistrationError> {
        let slot = self.slot.clone();
        let store = self.store.clone();
        let budget = self.stop_budget;
        target
            .registration()
            .reserve_cleanup("flags.presence.stopped")?
            .register(move || async move { stop(&store, &slot, budget).await });
        Ok(())
    }

    /// Registers `flags.presence` and `flags.presence_prune`, and returns
    /// their readers.
    pub(crate) fn register<T: RegistrationTarget + ?Sized>(
        self,
        target: &mut T,
        shared: Arc<Shared>,
    ) -> Result<(PeriodicReader, PeriodicReader), RegistrationError> {
        let Self {
            store,
            slot,
            floor,
            heartbeat,
            prune,
            ..
        } = self;
        let beat_store = store.clone();
        let presence = register_periodic_in(target, "flags.presence", heartbeat, move |context| {
            let store = beat_store.clone();
            let slot = slot.clone();
            let shared = shared.clone();
            let floor = floor.clone();
            async move {
                if shared.schema_is_behind() {
                    // Unknown columns would fail the heartbeat; the process
                    // stays out of the fleet view until the migration runs.
                    tracing::event!(
                        name: "flags.presence",
                        target: "batter::flags",
                        tracing::Level::DEBUG,
                        batter.flags.presence.cause = "schema_behind",
                        "flags heartbeat skipped",
                    );
                    return Ok(());
                }
                let report = report(&shared, floor);
                let mut slot = slot.lock().await;
                match &mut *slot {
                    Slot::Active(presence) => store
                        .heartbeat(presence, &report, work_deadline(context.deadline()))
                        .await
                        .map_err(|error| PeriodicFailure::recoverable(PresenceFailure(error))),
                    Slot::Stopped => Ok(()),
                }
            }
        })?;
        let limit = PruneLimit::new(PRUNE_ROWS).expect("the prune limit is in range");
        let pruning =
            register_periodic_in(target, "flags.presence_prune", prune, move |context| {
                let store = store.clone();
                async move {
                    store
                        .prune(limit, work_deadline(context.deadline()))
                        .await
                        .map(|_| ())
                        .map_err(|error| PeriodicFailure::recoverable(PresenceFailure(error)))
                }
            })?;
        Ok((presence, pruning))
    }
}

async fn stop(store: &PooledStore, slot: &Mutex<Slot>, budget: Duration) -> Result<(), BoxError> {
    let mut slot = slot.lock().await;
    let presence = match std::mem::replace(&mut *slot, Slot::Stopped) {
        Slot::Active(presence) => *presence,
        Slot::Stopped => return Ok(()),
    };
    match store.stop(presence, Instant::now() + budget - MARGIN).await {
        Ok(_) => Ok(()),
        Err(StopFailure::NotRecorded { error, presence }) => {
            // The process is ending either way; its row goes Silent at its
            // expiry and prune removes it after its retention.
            *slot = Slot::Active(presence);
            Err(Box::new(PresenceFailure(error)))
        }
        Err(failure @ StopFailure::Unknown { .. }) => Err(Box::new(failure)),
    }
}

/// What this heartbeat reports, all from one published record: what it
/// stands on and the keys it holds.
fn report(shared: &Shared, floor_engaged: KeySample) -> PresenceReport {
    let (snapshot, held) = match shared.reader().record() {
        None => (SnapshotReport::NotLoaded, KeySample::default()),
        Some(record) => {
            let snapshot = match record.authority().standing() {
                Standing::Unpublished => SnapshotReport::NotLoaded,
                Standing::Persisted { version } => SnapshotReport::PersistedFile(version),
                Standing::ControlPlane { version, .. } => SnapshotReport::ControlPlane(version),
            };
            // The holds a view of this same record captured now has, so a
            // stale view's extra holds are reported while it is stale and the
            // version is never paired with another publication's holds.
            let held = KeySample::of(
                shared
                    .reader()
                    .held_in(&record, Instant::now())
                    .into_iter()
                    .map(|(key, _)| key),
            );
            (snapshot, held)
        }
    };
    PresenceReport {
        snapshot,
        held,
        floor_engaged,
    }
}

/// The kill switches this process's settings hold engaged.
fn floor_engaged(base: &Base) -> Vec<FlagKey> {
    base.resolved()
        .filter(|resolution| {
            resolution.declared().kind() == Kind::Switch
                && resolution.source() == ResolutionSource::Literal
                && resolution.variant() == SwitchState::Engaged.as_str()
        })
        .map(|resolution| resolution.key().clone())
        .collect()
}

fn whole_seconds(duration: Duration) -> Duration {
    let seconds = duration.as_secs() + u64::from(duration.subsec_nanos() > 0);
    Duration::from_secs(seconds)
}

/// A presence or prune run that failed; never fatal.
#[derive(Debug, thiserror::Error)]
#[error("flags presence failed")]
struct PresenceFailure(#[source] StoreError);

impl From<RegistrationError> for RegisterError {
    fn from(error: RegistrationError) -> Self {
        Self::Registration(error)
    }
}
