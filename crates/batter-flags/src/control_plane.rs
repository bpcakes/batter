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
use batter_flags_core::low_level::Refusal;
use batter_flags_postgres::{PooledStore, ProcessId, StoreTiming};
use batter_sqlx::PgProfiledPool;
use tokio::time::Instant;

pub use self::{
    presence::{ApplicationName, BuildId, HeartbeatCadence, Presence, PresenceError},
    status::{Gate, LoadCause, RejectionCause, SnapshotStatus},
    timing::{LoadPolicy, RefreshTiming},
};
use self::{refresh::Persistence, source::LoadSource, status::Shared, timing::Budgets};
pub use crate::last_good::{
    DirectoryError, FileProblem, PersistFailure, SnapshotDirectory, SnapshotFilePath, StartupFile,
};
use crate::{
    Flags,
    declared::Resolved,
    last_good::{self, Initial, LastGood, Persister},
    reader::{Reader, Writer},
};

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
    startup_file: StartupFile,
}

/// The store a registered control plane reads and heartbeats through.
///
/// It is built only from a pool that [`batter_sqlx::pool_in`] registered,
/// so the pool's close was registered before the flags cleanup and, cleanup
/// running in reverse, closes after the stop marker has run.
#[derive(Clone, Debug)]
pub struct ControlPlaneStore(PooledStore);

impl ControlPlaneStore {
    /// The store over `pool`, under `timing`.
    pub fn new(pool: &PgProfiledPool, timing: StoreTiming) -> Self {
        Self(PooledStore::new(pool.pool().clone(), timing))
    }
}

impl Deref for RegisteredFlags {
    type Target = Flags;

    fn deref(&self) -> &Flags {
        &self.flags
    }
}

impl RegisteredFlags {
    /// The loader's status now, in [`SnapshotStatus`]'s precedence order.
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

    /// How many keys the served snapshot holds.
    pub fn held(&self) -> usize {
        self.shared
            .reader()
            .record()
            .map_or(0, |record| record.snapshot().held().count())
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

    /// The most recent persist that did not write, if there has been one.
    pub fn last_persist_failure(&self) -> Option<PersistFailure> {
        self.shared.last_persist_failure()
    }

    /// The heartbeat component's runs so far. A heartbeat skipped while the
    /// schema is behind succeeds without reaching the store.
    pub fn presence(&self) -> PeriodicSummary {
        self.presence.snapshot()
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
/// Every input is validated before anything is registered. The components
/// are registered in this order: the stop marker's cleanup slot, the
/// refresh, the heartbeat and the prune. Core has no all-or-nothing
/// registration, so a root that ignores an `Err` from a manual supervisor can
/// run a partly registered loader; `Startup::protected` prevents that.
pub fn register_in<T: RegistrationTarget + ?Sized>(
    target: &mut T,
    store: ControlPlaneStore,
    resolved: Resolved,
    policy: LoadPolicy,
    presence: Presence,
) -> Result<RegisteredFlags, RegisterError> {
    let source = LoadSource::Store(store.0.clone());
    register_with(target, source, &store.0, resolved, policy, presence)
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
    let presence = presence::prepare(store, presence, &base, class.name(), budgets)?;
    let scope = base.validation_scope();
    let (mut writer, reader) = Reader::live(
        base,
        policy.pre_publication(),
        timing.maximum_age(),
        Instant::now(),
    );
    let reader = Arc::new(reader);
    let shared = Arc::new(Shared::new(reader.clone()));
    let (persistence, startup_file) = match policy.last_good() {
        LastGood::Off => (Persistence::Off, StartupFile::Disabled),
        LastGood::File(directory) => (
            Persistence::File(Persister::new(directory.path().clone())),
            publish_file(&mut writer, &shared, directory),
        ),
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
    )?;
    let process = presence.process();
    presence
        .reserve_stop(target)
        .map_err(RegisterError::Registration)?;
    refresh
        .register(target)
        .map_err(RegisterError::Registration)?;
    let (presence, prune) = presence
        .register(target, shared.clone())
        .map_err(RegisterError::Registration)?;
    Ok(RegisteredFlags {
        flags: Flags::shared(reader),
        shared,
        process,
        presence,
        prune,
        startup_file,
    })
}

/// Publishes a usable last-good file before the driver starts, so the first
/// view of every surface sees it. A file that cannot be used is recorded and
/// publishes nothing.
fn publish_file(
    writer: &mut Writer,
    shared: &Shared,
    directory: &SnapshotDirectory,
) -> StartupFile {
    let bytes = match directory.initial() {
        Initial::Absent => return StartupFile::Missing,
        Initial::Unreadable => return StartupFile::Unusable(FileProblem::Unreadable),
        Initial::Read(bytes) => bytes,
    };
    match last_good::load(shared.reader().base(), bytes) {
        Ok(snapshot) => {
            let version = snapshot.version();
            writer.apply(Arc::new(snapshot), Instant::now());
            StartupFile::Published { version }
        }
        Err(problem) => StartupFile::Unusable(problem),
    }
}

#[cfg(test)]
mod tests;
