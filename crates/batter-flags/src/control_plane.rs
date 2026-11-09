//! The protected registration of the audited `PostgreSQL` control plane.
//!
//! [`register_in`] turns a [`Resolved`] into [`RegisteredFlags`]: it
//! registers the loader that reads, validates and publishes snapshots
//! (`flags.refresh`), the process heartbeat (`flags.presence`), the prune of
//! expired rows (`flags.presence_prune`) and the stop marker that runs before
//! the application's pool closes (`flags.presence.stopped`). Nothing else
//! publishes to the returned flags.
//!
//! What a process serves before and between publications is the root's
//! choice, made explicit: under [`LoadPolicy::unready_until_loaded`] it is not
//! ready until a control-plane read is published, while
//! [`LoadPolicy::start_on_defaults`] serves the declared base until then. A
//! readiness condition narrows readiness further: [`Gate::Published`] declines
//! the bare base but accepts a Stale publication, and [`Gate::Fresh`] accepts
//! only a recently confirmed one. Without a condition, staleness is visible in
//! [`RegisteredFlags::status`] and the `flags.load` events, and every view
//! applies each switch's stale rule once its data is older than the maximum
//! age.
//!
//! ```no_run
//! use std::time::Duration;
//!
//! use batter_core::{cleanup::CleanupBudget, lifecycle::{ShutdownBudget, Supervisor}};
//! use batter_flags::{
//!     control_plane::{
//!         ApplicationName, BuildId, ControlPlaneStore, Gate, HeartbeatCadence, LoadPolicy,
//!         Presence, RefreshTiming, register_in,
//!     },
//!     declared::Resolved,
//!     postgres::StoreTiming,
//! };
//! use batter_sqlx::{PgSessionProfile, pool_in};
//! use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
//!
//! # fn example(resolved: Resolved, options: PgConnectOptions, profile: PgSessionProfile)
//! #     -> Result<(), Box<dyn std::error::Error>> {
//! # let budget = ShutdownBudget::new(Duration::from_secs(1), Duration::from_secs(1),
//! #     Duration::from_secs(1), CleanupBudget::new(Duration::from_secs(1),
//! #     Duration::from_secs(1), Duration::from_secs(1))?)?;
//! let mut supervisor = Supervisor::new(budget);
//! // The pool's close is registered first, so the stop marker runs before it.
//! let pool = pool_in(supervisor.reserve_cleanup("database")?, PgPoolOptions::new(), options, profile)?;
//! let second = Duration::from_secs(1);
//! let store = ControlPlaneStore::new(&pool, StoreTiming::new(second, second * 2, second, second * 2)?);
//! let timing = RefreshTiming::new(Duration::from_secs(15), Duration::from_secs(60))?;
//! let presence = Presence::new(
//!     ApplicationName::new("service")?,
//!     BuildId::new("1.4.0+abc123")?,
//!     HeartbeatCadence::new(Duration::from_secs(15), Duration::from_secs(3_600))?,
//! );
//! let flags = register_in(
//!     &mut supervisor,
//!     store,
//!     resolved,
//!     LoadPolicy::unready_until_loaded(timing, Duration::from_secs(120)),
//!     presence,
//! )?;
//! let (condition, satisfied) = flags.readiness_condition(Gate::Published);
//! # let _ = (condition, satisfied);
//! # Ok(()) }
//! ```
//!
//! [`RegisteredFlags`] dereferences to [`Flags`], so it has the same surfaces
//! and no way to read a view directly:
//!
//! ```compile_fail,E0624
//! fn read(flags: &batter_flags::control_plane::RegisteredFlags) {
//!     let _view = flags.capture();
//! }
//! ```

mod listen;
mod presence;
mod refresh;
mod source;
mod status;
mod timing;

use std::{ops::Deref, sync::Arc, time::Duration};

use batter_core::{
    ConfigurationError, RegistrationError,
    periodic::{PeriodicReader, PeriodicSummary},
    readiness::ReadinessCondition,
    registration::RegistrationTarget,
};
use batter_flags_core::low_level::{Base, Publisher, Refusal, Restored, ValidationScope};
use batter_flags_postgres::{PooledStore, ProcessId, StoreTiming};
use batter_sqlx::PgProfiledPool;
use tokio::time::Instant;

pub use self::{
    listen::ListenEndpoint,
    presence::{ApplicationName, BuildId, HeartbeatCadence, Presence, PresenceError},
    status::{Gate, LatestRead, LoadCause, LoaderState, Served, SnapshotStatus},
    timing::{LoadPolicy, RefreshTiming, WakeSpacingTooLong},
};
use self::{refresh::Persistence, source::LoadSource, status::Shared, timing::Budgets};
pub use crate::last_good::{
    DirectoryError, FileProblem, LastGoodStatus, PersistFailure, SnapshotDirectory,
    SnapshotFilePath, StartupFile,
};
use crate::{
    Flags,
    declared::Resolved,
    last_good::{self, Initial, LastGood, Persister},
    reader::Reader,
};

/// The component and finalizer names the control plane registers.
pub(crate) const REFRESH: &str = "flags.refresh";
pub(crate) const LISTEN: &str = "flags.listen";
pub(crate) const PRESENCE: &str = "flags.presence";
pub(crate) const PRUNE: &str = "flags.presence_prune";
pub(crate) const STOP_MARKER: &str = "flags.presence.stopped";

/// Flags whose values a registered loader keeps current.
///
/// It dereferences to [`Flags`], the same surface declared mode has, and adds
/// the loader's status and the readiness condition built on it.
pub struct RegisteredFlags {
    flags: Flags,
    shared: Arc<Shared>,
    process: ProcessId,
    presence: PeriodicReader,
    prune: PeriodicReader,
    refresh: PeriodicReader,
    startup_file: StartupFile,
    last_good: LastGoodReader,
}

/// Where a registered loader's last-good status is read.
enum LastGoodReader {
    Disabled,
    File(last_good::StatusReader),
}

/// The store a registered control plane reads and heartbeats through.
///
/// [`register_in`] verifies that [`batter_sqlx::pool_in`] registered the pool
/// with that same target before it installs any flags work. Caller-owned pools,
/// other owners and extracted cleanup stacks are refused.
#[derive(Clone, Debug)]
pub struct ControlPlaneStore {
    store: PooledStore,
    pool: PgProfiledPool,
}

impl ControlPlaneStore {
    /// The store over `pool`, under `timing`.
    pub fn new(pool: &PgProfiledPool, timing: StoreTiming) -> Self {
        Self {
            store: PooledStore::new(pool.pool().clone(), timing),
            pool: pool.clone(),
        }
    }
}

impl Deref for RegisteredFlags {
    type Target = Flags;

    fn deref(&self) -> &Flags {
        &self.flags
    }
}

impl RegisteredFlags {
    /// The loader's status now: whether it runs, what views serve and what
    /// the latest judged read did, read together.
    pub fn status(&self) -> SnapshotStatus {
        self.shared.status(Instant::now())
    }

    /// The readiness condition for `gate`, named `flags.snapshot` for either,
    /// to pass to `ReadinessEvaluator::with_condition`. [`Gate::Fresh`]
    /// implies [`Gate::Published`], so a root installs one, and adds it first,
    /// because the first unsatisfied condition is the one reported.
    pub fn readiness_condition(
        &self,
        gate: Gate,
    ) -> (
        ReadinessCondition,
        impl Fn() -> bool + Send + Sync + 'static,
    ) {
        let shared = self.shared.clone();
        (Shared::condition(), move || shared.satisfies(gate))
    }

    /// The most recent refusal of a read, if there has been one. It stays
    /// after a later publication clears the status.
    pub fn last_rejection(&self) -> Option<Refusal> {
        self.shared.last_rejection()
    }

    /// How many keys a view captured now holds: the served snapshot's holds
    /// for the view's freshness.
    pub fn held(&self) -> usize {
        self.shared.live().held_at(Instant::now()).len()
    }

    /// This process's id in the fleet view.
    pub fn process(&self) -> ProcessId {
        self.process
    }

    /// What the last-good file did at startup: disabled, missing, published
    /// or unusable, and why.
    pub fn startup_file(&self) -> StartupFile {
        self.startup_file
    }

    /// What last-good persistence is doing now: disabled, up to date, a
    /// publication waiting and why its last write failed, a write in flight,
    /// or a publication too large for the file.
    ///
    /// Migration: `last_persist_failure()` returned the last failure forever,
    /// even after a later write succeeded; match `LastGoodStatus::Waiting {
    /// last_failure, .. }` for a failure that still matters.
    pub fn last_good(&self) -> LastGoodStatus {
        match &self.last_good {
            LastGoodReader::Disabled => LastGoodStatus::Disabled,
            LastGoodReader::File(reader) => reader.status(),
        }
    }

    /// The heartbeat component's runs so far. A heartbeat skipped while the
    /// schema is behind succeeds without reaching the store.
    pub fn presence(&self) -> PeriodicSummary {
        self.presence.snapshot()
    }

    /// The refresh component's runs so far, woken ones included.
    pub fn refresh(&self) -> PeriodicSummary {
        self.refresh.snapshot()
    }

    /// The prune component's runs so far.
    pub fn prune(&self) -> PeriodicSummary {
        self.prune.snapshot()
    }
}

impl std::fmt::Debug for RegisteredFlags {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RegisteredFlags")
            .field("status", &self.status())
            .finish_non_exhaustive()
    }
}

/// Why registration refused its inputs, or could not register.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RegisterError {
    /// The pool's finalizer is not owned by this registration target.
    #[error("the flags pool must be registered in this target's cleanup stack")]
    PoolOwnership,
    /// The maximum age cannot cover one poll and one run budget, so a
    /// cadence whose runs all succeed would still go Stale.
    #[error("the flags maximum age must be at least {minimum:?}")]
    MaximumAgeTooShort {
        /// The shortest maximum age the store's timing allows.
        minimum: Duration,
    },
    /// The `unready_until_loaded` allowance cannot fit the first attempt and
    /// one full retry.
    #[error("the flags load allowance must be at least {minimum:?}")]
    AllowanceTooShort {
        /// The shortest allowance the cadence and the store's timing allow.
        minimum: Duration,
    },
    /// The declared base breaks a cross-flag invariant once a governing kill
    /// switch overrides a flag to its code default, a combination no hold
    /// could meet, since the control plane can engage the switch at any time.
    /// The settings name a literal the invariant cannot keep under the
    /// override; change the settings or the catalog.
    #[error(
        "the flags settings for {flag} break an invariant with {other} while a governing switch is engaged"
    )]
    GovernedInvariant {
        /// The flag the invariant is declared on.
        flag: batter_flags_core::FlagKey,
        /// The other flag.
        other: batter_flags_core::FlagKey,
        /// The invariant.
        invariant: Box<batter_flags_core::Invariant>,
    },
    /// A presence input is out of range.
    #[error(transparent)]
    Presence(#[from] PresenceError),
    /// A periodic policy derived from the inputs is invalid.
    #[error(transparent)]
    Timing(#[from] ConfigurationError),
    /// A component or cleanup name is taken: `register_in` runs once per
    /// supervisor.
    #[error(transparent)]
    Registration(RegistrationError),
}

/// Registers the control plane for `resolved` and returns its flags.
///
/// Every input is validated, and every component and finalizer name checked,
/// before anything is registered, so registration installs the whole loader
/// or nothing: the stop marker's cleanup slot, the refresh, the listener when
/// one is configured, the heartbeat and the prune.
///
/// Migration: a taken name, such as an application component already named
/// `flags.presence`, used to be refused after the stop marker, the refresh and
/// the listener were installed.
pub fn register_in<T: RegistrationTarget + ?Sized>(
    target: &mut T,
    store: ControlPlaneStore,
    resolved: Resolved,
    policy: LoadPolicy,
    presence: Presence,
) -> Result<RegisteredFlags, RegisterError> {
    let mut target = target.registration();
    if !store.pool.is_registered_in(&target) {
        return Err(RegisterError::PoolOwnership);
    }
    let source = LoadSource::Store(store.store.clone());
    register_with(
        &mut target,
        source,
        &store.store,
        resolved,
        policy,
        presence,
    )
}

fn register_with<T: RegistrationTarget + ?Sized>(
    target: &mut T,
    source: LoadSource,
    store: &PooledStore,
    resolved: Resolved,
    policy: LoadPolicy,
    presence: Presence,
) -> Result<RegisteredFlags, RegisterError> {
    // Validate everything before registering anything.
    let budgets = Budgets::for_policy(store.timing(), &policy);
    let startup = policy.startup(budgets)?;
    let timing = policy.timing();
    let (base, class) = resolved.into_parts();
    if let Some(invariant) = base.governed_invariant_breaks().into_iter().next() {
        return Err(RegisterError::GovernedInvariant {
            flag: invariant.flag().clone(),
            other: invariant.other().clone(),
            invariant: Box::new(invariant),
        });
    }
    let presence = presence::prepare(store, presence, &base, class.name(), budgets)?;
    let scope = base.validation_scope();
    let registered = Instant::now();
    let (persistence, startup_file, publisher, last_good) = match policy.last_good() {
        LastGood::Off => (
            Persistence::Off,
            StartupFile::Disabled,
            Publisher::new(&scope, registered),
            LastGoodReader::Disabled,
        ),
        LastGood::File(directory) => {
            let (publisher, startup_file) = restore_file(&base, &scope, registered, directory);
            let persister = Persister::new(directory.path().clone());
            let reader = LastGoodReader::File(persister.reader());
            (
                Persistence::File(persister),
                startup_file,
                publisher,
                reader,
            )
        }
    };
    let (writer, reader, live) = Reader::publishing(
        base,
        policy.pre_publication(),
        timing.maximum_age(),
        publisher,
    );
    let reader = Arc::new(reader);
    let shared = Arc::new(Shared::new(reader.clone(), live));
    // `LoadPolicy::with_listen` already refused a spacing longer than the poll.
    let listening = match policy.listen() {
        timing::Listen::Off => None,
        timing::Listen::On(endpoint) => Some((**endpoint).clone()),
    };
    let refresh = refresh::prepare(
        source,
        writer,
        scope,
        shared.clone(),
        persistence,
        timing.poll_interval(),
        budgets.refresh,
        startup,
        listening.as_ref().map(ListenEndpoint::wake_spacing),
    )?;
    // Every name is checked while this exclusive authority is held, so the
    // registrations below cannot be refused part way.
    let components: &[&'static str] = match &listening {
        Some(_) => &[REFRESH, LISTEN, PRESENCE, PRUNE],
        None => &[REFRESH, PRESENCE, PRUNE],
    };
    target
        .registration()
        .check_names(components, &[STOP_MARKER])
        .map_err(RegisterError::Registration)?;
    let process = presence.process();
    // Cleanup runs in reverse: the stop marker, then the listener's pool, then
    // the application's pool, which the root registered first.
    let mut presence = presence
        .reserve_stop(target)
        .map_err(RegisterError::Registration)?;
    let registered = refresh
        .register(&mut presence.registration())
        .map_err(RegisterError::Registration)?;
    if let (Some(endpoint), refresh::Registered::Woken(_, wake)) = (listening, &registered) {
        listen::register(&mut presence.registration(), endpoint, wake.clone())
            .map_err(RegisterError::Registration)?;
    }
    let (presence, prune) = presence
        .register(shared.clone())
        .map_err(RegisterError::Registration)?;
    Ok(RegisteredFlags {
        flags: Flags::shared(reader),
        shared,
        process,
        presence,
        prune,
        refresh: registered.reader(),
        startup_file,
        last_good,
    })
}

/// The publisher, restored from a usable last-good file before the driver
/// starts, so the first view of every surface sees it. A file that cannot be
/// used is recorded and publishes nothing.
fn restore_file(
    base: &Base,
    scope: &ValidationScope,
    registered: Instant,
    directory: &SnapshotDirectory,
) -> (Publisher<Instant>, StartupFile) {
    let unusable = |problem| {
        (
            Publisher::new(scope, registered),
            StartupFile::Unusable(problem),
        )
    };
    let bytes = match directory.initial() {
        Initial::Absent => return (Publisher::new(scope, registered), StartupFile::Missing),
        Initial::Unreadable => return unusable(FileProblem::Unreadable),
        Initial::Read(bytes) => bytes,
    };
    match last_good::read_file(base, bytes) {
        Ok(file) => {
            let version = file.version;
            let (publisher, restored) = Publisher::restore(scope, registered, version, file.rows);
            let startup_file = match restored {
                Restored::Published => StartupFile::Published { version },
                Restored::Rejected(rejection) => {
                    StartupFile::Unusable(FileProblem::Rejected(rejection))
                }
            };
            (publisher, startup_file)
        }
        Err(problem) => unusable(problem),
    }
}

#[cfg(test)]
mod tests;
