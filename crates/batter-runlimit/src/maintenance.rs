//! PostgreSQL limiters whose expired-row cleanup is a registered component.
//!
//! Native PostgreSQL storage keeps an expired counter, and its capacity slot,
//! until a bounded `cleanup_expired` call deletes it. [`register_in`] is the
//! canonical way to construct a supervised PostgreSQL limiter: it registers that
//! cleanup as a periodic component through the supplied registration authority
//! and only then returns the [`MaintainedLimiter`] the application serves with,
//! so the protected path cannot produce a PostgreSQL limiter whose storage is
//! never cleaned.
//!
//! [`crate::postgres::PostgresLimiter`] and
//! [`crate::postgres::PostgresGcraLimiter`] remain reachable as the native,
//! **unmaintained** low-level form. A store built from them alone keeps every
//! expired row until its caller schedules cleanup itself, and a full capacity
//! shard then denies new keys. Use them directly only for setup such as
//! migration, for a finite command, or under a cleanup schedule the caller owns.
//!
//! # What runs
//!
//! Each run calls the native `cleanup_expired` once, on the limiter's own pool,
//! login and configuration, deleting at most
//! [`ExpiryMaintenance::maximum_rows`] expired rows. Runs follow
//! [`batter_core::periodic`] semantics: one serial schedule, an immediate first
//! run, missed ticks skipped, and drain observed between and during runs. Every
//! native maintenance failure is retained as a recoverable failure and the next
//! scheduled run still happens; maintenance never escalates to a process drain.
//! Removed-row counts reach a native observer installed with `with_observer`.
//!
//! Each run's budget is derived, never chosen: the limiter's pool-acquisition
//! timeout plus its operation timeout plus one second. Native cleanup applies
//! both of those bounds itself, so during ordinary running a slow acquisition
//! or statement reports its native [`MaintenanceError`] before that budget
//! expires. An earlier startup allowance or shutdown boundary can still destroy
//! the run before native classification; initialization expiry and stopping
//! retain the existing [`batter_core::periodic`] outcomes instead.
//!
//! Every stopping class ends before finalizers run, so maintenance registered
//! here has stopped, and an active run has been destroyed, before a pool closed
//! by a cleanup finalizer such as `batter_sqlx::pool_in` begins to close.
//!
//! # Grants
//!
//! Maintenance runs under the limiter's login. That login needs the admission
//! selection and the expiry-cleanup selection of its store from
//! [`crate::grants`]: `FixedWindowAdmission` with `FixedWindowExpiryCleanup`, or
//! `GcraAdmission` with `GcraExpiryCleanup`. Neither cleanup selection needs
//! relation-level `SELECT`. A cleanup transaction checks its privileges even
//! when nothing has expired, so [`PeriodicStartup::after_first_success`] withholds
//! readiness until one cleanup has committed under that login. With
//! [`PeriodicStartup::immediate`], a missing grant is retained only as recurring
//! recoverable failures while the process serves.
//!
//! # Limits
//!
//! Registration proves neither that the schema is installed nor that rows were
//! removed: cleanup is bounded per run, skips rows locked by concurrent
//! admission, and an uncertain commit
//! ([`MaintenanceError::may_have_removed_rows`]) may or may not have deleted
//! rows. The row bound limits deletions, not the planner's scan cost; a forced
//! generic plan can scan the counter table. Throughput is at most `maximum_rows`
//! per interval. Migrations, pools
//! and their ownership remain the application's.
//!
//! ```no_run
//! use batter_core::{
//!     BoxError,
//!     periodic::{PeriodicShutdown, PeriodicStartup},
//!     startup::ProtectedStartupScope,
//! };
//! use batter_runlimit::{
//!     Quota,
//!     maintenance::{ExpiryMaintenance, MaintainedLimiter, register_in},
//!     postgres::PostgresLimiter,
//! };
//! use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
//! use std::{num::NonZeroU32, time::Duration};
//!
//! const ROWS_PER_RUN: NonZeroU32 = NonZeroU32::new(1_000).unwrap();
//!
//! async fn quota(
//!     scope: &mut ProtectedStartupScope,
//!     connect: PgConnectOptions,
//! ) -> Result<Quota<MaintainedLimiter<PostgresLimiter>>, BoxError> {
//!     // The pool's finalizer runs only after maintenance has stopped.
//!     let pool = batter_sqlx::pool_in(
//!         scope.reserve_cleanup("quota.pool")?,
//!         PgPoolOptions::new().max_connections(4),
//!         connect,
//!     );
//!     let limiter = PostgresLimiter::new(pool);
//!     limiter.migrate().await?;
//!     let limiter = register_in(
//!         scope,
//!         "quota.maintenance",
//!         limiter,
//!         ExpiryMaintenance::new(
//!             Duration::from_secs(30),
//!             ROWS_PER_RUN,
//!             PeriodicStartup::after_first_success(Duration::from_secs(30))?,
//!             PeriodicShutdown::StopAtDrain,
//!         )?,
//!     )?;
//!     Ok(Quota::new(limiter))
//! }
//! ```

use batter_core::{
    ConfigurationError, RegistrationError,
    periodic::{
        PeriodicFailure, PeriodicPolicy, PeriodicReader, PeriodicShutdown, PeriodicStartup,
        register_periodic_in,
    },
    registration::RegistrationTarget,
};
use runlimit_core::{BatchDecision, Check, Decision, Limiter};
use runlimit_postgres::{MaintenanceError, PostgresConfig, PostgresGcraLimiter, PostgresLimiter};
use std::{fmt, future::Future, num::NonZeroU32, time::Duration};

/// How far each run's budget exceeds the native acquisition and operation bounds.
const NATIVE_CLASSIFICATION_MARGIN: Duration = Duration::from_secs(1);

/// A validated cleanup schedule for one maintained PostgreSQL limiter.
///
/// The interval, startup and stopping class mean exactly what they mean for
/// [`PeriodicPolicy`]. The per-run budget is not a field: [`register_in`]
/// derives it from the limiter's own native configuration.
///
/// A zero row bound is unrepresentable. The native call treats zero as "do
/// nothing", which would register a schedule that never cleans.
///
/// ```
/// use batter_core::{
///     ConfigurationError,
///     periodic::{PeriodicShutdown, PeriodicStartup},
/// };
/// use batter_runlimit::maintenance::ExpiryMaintenance;
/// use std::{num::NonZeroU32, time::Duration};
///
/// let rows = NonZeroU32::new(500).ok_or("nonzero")?;
/// let maintenance = ExpiryMaintenance::new(
///     Duration::from_secs(30),
///     rows,
///     PeriodicStartup::immediate(),
///     PeriodicShutdown::StopAtDrain,
/// )?;
/// assert_eq!(maintenance.maximum_rows(), rows);
///
/// // A zero interval is rejected before registration, not at the first tick.
/// assert_eq!(
///     ExpiryMaintenance::new(
///         Duration::ZERO,
///         rows,
///         PeriodicStartup::immediate(),
///         PeriodicShutdown::StopAtDrain,
///     ),
///     Err(ConfigurationError::Zero("periodic interval")),
/// );
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExpiryMaintenance {
    interval: Duration,
    maximum_rows: NonZeroU32,
    startup: PeriodicStartup,
    shutdown: PeriodicShutdown,
}

impl ExpiryMaintenance {
    /// Validate a cleanup schedule.
    ///
    /// # Errors
    ///
    /// Returns the same [`ConfigurationError`] as [`PeriodicPolicy::new`] for a
    /// zero or unrepresentable interval.
    pub fn new(
        interval: Duration,
        maximum_rows: NonZeroU32,
        startup: PeriodicStartup,
        shutdown: PeriodicShutdown,
    ) -> Result<Self, ConfigurationError> {
        // The interval doubles as a placeholder budget: this applies exactly the
        // validation registration later repeats with the derived budget.
        PeriodicPolicy::new(interval, interval, startup, shutdown)?;
        Ok(Self {
            interval,
            maximum_rows,
            startup,
            shutdown,
        })
    }

    /// Fixed interval between cleanup starts, with missed ticks skipped.
    pub fn interval(&self) -> Duration {
        self.interval
    }

    /// Most expired rows one run may delete.
    pub fn maximum_rows(&self) -> NonZeroU32 {
        self.maximum_rows
    }

    /// Selected startup acknowledgement policy.
    pub fn startup(&self) -> PeriodicStartup {
        self.startup
    }

    /// Selected library-owned stopping class.
    pub fn shutdown(&self) -> PeriodicShutdown {
        self.shutdown
    }

    fn policy(&self, config: PostgresConfig) -> PeriodicPolicy {
        let budget = config.pool_acquire_timeout()
            + config.operation_timeout()
            + NATIVE_CLASSIFICATION_MARGIN;
        PeriodicPolicy::new(self.interval, budget, self.startup, self.shutdown)
            .expect("a validated interval and a native budget of at most two minutes are valid")
    }
}

mod private {
    use runlimit_postgres::{MaintenanceError, PostgresConfig};
    use std::future::Future;

    pub trait Sealed {
        fn native_config(&self) -> PostgresConfig;

        fn cleanup(
            &self,
            maximum_rows: u32,
        ) -> impl Future<Output = Result<u64, MaintenanceError>> + Send;
    }
}

/// A native PostgreSQL limiter whose storage needs bounded expiry cleanup.
///
/// This trait is sealed. It is implemented for
/// [`crate::postgres::PostgresLimiter`] and
/// [`crate::postgres::PostgresGcraLimiter`], and only names which limiters
/// [`register_in`] accepts.
pub trait ExpiringStore: private::Sealed + Limiter + Clone + 'static {}

impl private::Sealed for PostgresLimiter {
    fn native_config(&self) -> PostgresConfig {
        self.config()
    }

    fn cleanup(
        &self,
        maximum_rows: u32,
    ) -> impl Future<Output = Result<u64, MaintenanceError>> + Send {
        PostgresLimiter::cleanup_expired(self, maximum_rows)
    }
}

impl ExpiringStore for PostgresLimiter {}

impl private::Sealed for PostgresGcraLimiter {
    fn native_config(&self) -> PostgresConfig {
        self.config()
    }

    fn cleanup(
        &self,
        maximum_rows: u32,
    ) -> impl Future<Output = Result<u64, MaintenanceError>> + Send {
        PostgresGcraLimiter::cleanup_expired(self, maximum_rows)
    }
}

impl ExpiringStore for PostgresGcraLimiter {}

/// A PostgreSQL limiter whose expiry cleanup is a registered periodic component.
///
/// Only [`register_in`] constructs it, so holding one is evidence that cleanup
/// was registered with the same native limiter. It implements
/// [`runlimit_core::Limiter`] by delegating each check unchanged, so it serves
/// through [`crate::Quota::new`] and the HTTP assembly exactly as the native
/// limiter would. Cloning shares the same storage and registered maintenance.
///
/// It is not evidence that maintenance is running or succeeding: read
/// [`Self::maintenance`] or the process completion report.
#[derive(Clone)]
pub struct MaintainedLimiter<L> {
    limiter: L,
    maintenance: PeriodicReader,
}

impl<L> MaintainedLimiter<L> {
    /// Read-only evidence of the registered cleanup runs.
    ///
    /// The process completion report retains the same evidence, so reading
    /// this is never required.
    pub fn maintenance(&self) -> &PeriodicReader {
        &self.maintenance
    }
}

impl<L: fmt::Debug> fmt::Debug for MaintainedLimiter<L> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MaintainedLimiter")
            .field("limiter", &self.limiter)
            .finish_non_exhaustive()
    }
}

impl<L: Limiter> Limiter for MaintainedLimiter<L> {
    type Policy = L::Policy;
    type CheckError = L::CheckError;
    type CheckAllError = L::CheckAllError;

    fn check(
        &self,
        check: &Check<'_, Self::Policy>,
    ) -> impl Future<Output = Result<Decision, Self::CheckError>> + Send {
        self.limiter.check(check)
    }

    fn check_all(
        &self,
        checks: &[Check<'_, Self::Policy>],
    ) -> impl Future<Output = Result<BatchDecision, Self::CheckAllError>> + Send {
        self.limiter.check_all(checks)
    }
}

/// Register a native PostgreSQL limiter's expiry cleanup and return the limiter
/// to serve with.
///
/// Registration is inert and validated exactly as for
/// [`register_periodic_in`]: an invalid or duplicate name is rejected, and
/// nothing touches the database until the supervisor starts the component. The
/// limiter is consumed either way. See the [module documentation](self) for
/// what each run does, its derived budget, the grants its login needs and the
/// low-level unmaintained form.
///
/// ```
/// use batter_core::{
///     cleanup::CleanupBudget,
///     lifecycle::{ShutdownBudget, Supervisor},
///     periodic::{PeriodicShutdown, PeriodicStartup},
/// };
/// use batter_runlimit::{
///     Quota,
///     maintenance::{ExpiryMaintenance, register_in},
///     postgres::PostgresLimiter,
/// };
/// use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
/// use std::{num::NonZeroU32, time::Duration};
///
/// # #[tokio::main(flavor = "current_thread")]
/// # async fn main() -> Result<(), batter_core::BoxError> {
/// let second = Duration::from_secs(1);
/// let mut supervisor = Supervisor::new(ShutdownBudget::new(
///     second,
///     second,
///     second,
///     CleanupBudget::new(second, second, second)?,
/// )?);
/// // A lazy pool: registration performs no database work.
/// let pool = PgPoolOptions::new().connect_lazy_with(PgConnectOptions::new());
/// let limiter = register_in(
///     &mut supervisor,
///     "quota.maintenance",
///     PostgresLimiter::new(pool),
///     ExpiryMaintenance::new(
///         Duration::from_secs(30),
///         NonZeroU32::new(1_000).ok_or("nonzero")?,
///         PeriodicStartup::immediate(),
///         PeriodicShutdown::StopAtDrain,
///     )?,
/// )?;
/// assert_eq!(limiter.maintenance().snapshot().invocations, 0);
/// let _quota = Quota::new(limiter);
/// # Ok(()) }
/// ```
///
/// # Errors
///
/// Returns [`RegistrationError`] for an invalid or already registered name.
pub fn register_in<T, L>(
    target: &mut T,
    name: &'static str,
    limiter: L,
    maintenance: ExpiryMaintenance,
) -> Result<MaintainedLimiter<L>, RegistrationError>
where
    T: RegistrationTarget + ?Sized,
    L: ExpiringStore,
{
    let policy = maintenance.policy(limiter.native_config());
    let maximum_rows = maintenance.maximum_rows.get();
    let store = limiter.clone();
    let reader = register_periodic_in(target, name, policy, move |_run| {
        let store = store.clone();
        async move {
            private::Sealed::cleanup(&store, maximum_rows)
                .await
                .map(drop)
                .map_err(PeriodicFailure::recoverable)
        }
    })?;
    Ok(MaintainedLimiter {
        limiter,
        maintenance: reader,
    })
}
