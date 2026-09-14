//! Purpose-qualified application settings and inert native preparation.
//!
//! Defaults < selected literal file < captured environment < explicit overrides.
//! Dedicated sources reject unknown keys. Environment ignores unrelated names,
//! rejects unknown BATTER_/JOBS_ names and all PG* names (including empty values).
//! Maintenance also ignores known serving-only names from captured environment,
//! so an operator can run it from a serving deployment without parsing those values.
//! Selected files are limited to 64 KiB; no file is discovered implicitly.
//!
//! [`crate::config::ServingSettings`] requires a password-qualified PostgreSQL endpoint,
//! concrete authentication, a validated native worker configuration, and validated
//! local capacities and budgets. [`crate::config::MaintenanceSettings`] recognizes only its database
//! schema and permits passwordless TCP endpoints. There is no conversion from
//! maintenance into serving. Loading and preparation perform no acquisition,
//! binding, migration, or task spawning.
//!
//! ```no_run
//! use batter::settings::SettingsSource;
//! use batter_example_reference_service::{config::ServingSettings, runtime};
//!
//! let overrides = SettingsSource::from_pairs([
//!     ("DATABASE_URL".into(), "postgres://user:fake@localhost/db?sslmode=disable".into()),
//!     ("JOBS_WORKER_ID".into(), "worker-example".into()),
//!     ("BATTER_AUTH_OWNER_ID".into(), "00000000-0000-0000-0000-000000000001".into()),
//!     ("BATTER_AUTH_TOKEN".into(), "fake-example-token".into()),
//! ])?;
//! let settings = ServingSettings::from_sources(
//!     None,
//!     SettingsSource::default(),
//!     overrides,
//! )?;
//! let prepared = runtime::prepare(settings)?;
//! let future = runtime::run(prepared);
//! drop(future); // never polling the future starts no application work
//! # Ok::<(), batter::settings::SettingsError>(())
//! ```

mod endpoint;
mod pool;
mod worker;
pub use pool::PoolSettings;
pub use worker::WorkerSettings;

use crate::{auth::BearerAuthenticator, delivery::OwnerId};
use batter::{
    admission::{Bulkhead, BulkheadCapacity},
    lifecycle::{OperationAdmission, ProcessCapacity, ShutdownBudget, Supervisor},
    settings::{SettingsError, SettingsSource, bounded_u64, milliseconds, read_file},
};
use batter_axum::{RequestPolicy, ResponseConstructionBudget};
use runledger_runtime::config::JobsConfig;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use std::{fmt, net::SocketAddr, path::Path, time::Duration};

const MAX_DURATION: Duration = Duration::from_secs(365 * 24 * 60 * 60);
const SERVING_ROOT_NAMES: &[&str] = &[
    "BATTER_BIND",
    "BATTER_REQUEST_TIMEOUT_MS",
    "BATTER_BULKHEAD_CAPACITY",
    "BATTER_PROCESS_CAPACITY",
    "BATTER_AUTH_OWNER_ID",
    "BATTER_AUTH_TOKEN",
    "DATABASE_URL",
];
const MAINTENANCE_ROOT_NAMES: &[&str] = &["DATABASE_URL"];
const RESERVED_ENVIRONMENT_PREFIXES: &[&str] = &["BATTER_", "JOBS_", "PG"];

fn process_sources(
    file: Option<&Path>,
) -> Result<(Option<SettingsSource>, SettingsSource), SettingsError> {
    let environment = SettingsSource::from_pairs(std::env::vars_os())?;
    let file = file.map(|path| read_file(path, 64 * 1024)).transpose()?;
    Ok((file, environment))
}

fn merge_sources(
    names: &[&str],
    environment_names: &[&str],
    file: Option<SettingsSource>,
    environment: SettingsSource,
    overrides: SettingsSource,
) -> Result<SettingsSource, SettingsError> {
    let mut values = SettingsSource::default();
    if let Some(file) = file {
        values.overlay(file.select(names, &[], false)?);
    }
    let environment = environment.select(environment_names, RESERVED_ENVIRONMENT_PREFIXES, true)?;
    values.overlay(environment.select(names, &[], true)?);
    values.overlay(overrides.select(names, &[], false)?);
    Ok(values)
}

fn serving_names() -> Vec<&'static str> {
    SERVING_ROOT_NAMES
        .iter()
        .chain(pool::NAMES)
        .chain(worker::NAMES)
        .copied()
        .collect()
}

fn capacity(values: &SettingsSource, name: &'static str) -> Result<usize, SettingsError> {
    usize::try_from(bounded_u64(
        values.text(name)?.unwrap_or("32"),
        name,
        1,
        tokio::sync::Semaphore::MAX_PERMITS as u64,
    )?)
    .map_err(|error| SettingsError::new(name, "integer overflow").with_cause(error))
}

fn capacities(
    values: &SettingsSource,
) -> Result<(BulkheadCapacity, ProcessCapacity), SettingsError> {
    let bulkhead =
        BulkheadCapacity::new(capacity(values, "BATTER_BULKHEAD_CAPACITY")?).map_err(|error| {
            SettingsError::new("BATTER_BULKHEAD_CAPACITY", "native validation failed")
                .with_cause(error)
        })?;
    let process =
        ProcessCapacity::new(capacity(values, "BATTER_PROCESS_CAPACITY")?).map_err(|error| {
            SettingsError::new("BATTER_PROCESS_CAPACITY", "native validation failed")
                .with_cause(error)
        })?;
    Ok((bulkhead, process))
}

fn authenticator(values: &SettingsSource) -> Result<BearerAuthenticator, SettingsError> {
    let owner = values
        .required("BATTER_AUTH_OWNER_ID")?
        .parse()
        .map_err(|error| {
            SettingsError::new("BATTER_AUTH_OWNER_ID", "invalid UUID").with_cause(error)
        })?;
    let owner = OwnerId::new(owner).map_err(|error| {
        SettingsError::new("BATTER_AUTH_OWNER_ID", "invalid UUID").with_cause(error)
    })?;
    let token =
        batter::settings::SecretString::new(values.required("BATTER_AUTH_TOKEN")?.to_owned());
    BearerAuthenticator::new(owner, token)
        .map_err(|error| SettingsError::new("BATTER_AUTH_TOKEN", "invalid token").with_cause(error))
}

/// Validated inputs for the serving process.
///
/// Every field needed by HTTP, PostgreSQL, finite-process work, and the native
/// worker is concrete. Construction cannot produce a maintenance-qualified value.
pub struct ServingSettings {
    bind: SocketAddr,
    request_budget: ResponseConstructionBudget,
    bulkhead_capacity: BulkheadCapacity,
    process_capacity: ProcessCapacity,
    pool: PoolSettings,
    jobs: JobsConfig,
    endpoint: endpoint::ServingEndpoint,
    authenticator: BearerAuthenticator,
}

impl ServingSettings {
    /// Capture process environment once, read only an explicitly selected file,
    /// then apply explicit overrides. Selected-file failures never fall back.
    pub fn from_process(
        file: Option<&Path>,
        overrides: SettingsSource,
    ) -> Result<Self, SettingsError> {
        let (file, environment) = process_sources(file)?;
        Self::from_sources(file, environment, overrides)
    }

    /// Load injected serving sources without consulting process globals.
    pub fn from_sources(
        file: Option<SettingsSource>,
        environment: SettingsSource,
        overrides: SettingsSource,
    ) -> Result<Self, SettingsError> {
        let names = serving_names();
        let values = merge_sources(&names, &names, file, environment, overrides)?;
        let bind = values
            .text("BATTER_BIND")?
            .unwrap_or("127.0.0.1:3000")
            .parse()
            .map_err(|error| {
                SettingsError::new("BATTER_BIND", "invalid address").with_cause(error)
            })?;
        let request_budget = milliseconds(
            values.text("BATTER_REQUEST_TIMEOUT_MS")?.unwrap_or("2000"),
            "BATTER_REQUEST_TIMEOUT_MS",
            MAX_DURATION,
        )?;
        let request_budget = ResponseConstructionBudget::new(request_budget).map_err(|error| {
            SettingsError::new("BATTER_REQUEST_TIMEOUT_MS", "native validation failed")
                .with_cause(error)
        })?;
        let (bulkhead_capacity, process_capacity) = capacities(&values)?;
        Ok(Self {
            bind,
            request_budget,
            bulkhead_capacity,
            process_capacity,
            pool: PoolSettings::from_values(&values)?,
            jobs: WorkerSettings::from_values(&values)?.into_jobs_config(),
            endpoint: endpoint::ServingEndpoint::parse(values.required("DATABASE_URL")?)?,
            authenticator: authenticator(&values)?,
        })
    }

    /// Return the validated bind address. Binding remains a runtime action.
    pub fn bind(&self) -> SocketAddr {
        self.bind
    }

    /// Construct all native serving inputs without connecting, binding, or spawning.
    ///
    /// SQLx reads ambient defaults during native option construction even though
    /// passfile lookup is disabled. This boundary rejects any current PG* entry;
    /// callers must not mutate the environment during preparation.
    pub(crate) fn prepare(
        self,
        shutdown_budget: ShutdownBudget,
    ) -> Result<PreparedServing, SettingsError> {
        let connect_options = self.endpoint.connect_options_from_process()?;
        Ok(PreparedServing {
            bind: self.bind,
            pool_options: self.pool.pool_options(),
            connect_options,
            supervisor: Supervisor::with_process_capacity(shutdown_budget, self.process_capacity),
            jobs: self.jobs,
            http: PreparedHttp {
                request_budget: self.request_budget,
                bulkhead_capacity: self.bulkhead_capacity,
                authenticator: self.authenticator,
            },
        })
    }

    /// Prepare only the serving-qualified HTTP inputs for router tests/adapters.
    /// This performs no I/O and cannot be constructed from maintenance settings.
    pub fn prepare_http(&self) -> PreparedHttp {
        PreparedHttp {
            request_budget: self.request_budget,
            bulkhead_capacity: self.bulkhead_capacity,
            authenticator: self.authenticator.clone(),
        }
    }

    /// Construct a native response policy for focused configuration probes.
    /// The protected serving root uses [`PreparedHttp`] instead.
    pub fn request_policy(&self, admission: OperationAdmission) -> RequestPolicy {
        RequestPolicy::new(admission, self.request_budget)
    }

    /// Construct process-local admission for focused configuration probes.
    /// The protected serving root uses [`PreparedHttp`] instead.
    pub fn bulkhead(&self) -> Bulkhead {
        Bulkhead::new(self.bulkhead_capacity)
    }

    /// Construct a finite-process supervisor for focused configuration probes.
    /// The protected serving root consumes [`PreparedServing`] instead.
    /// A direct caller must drive the running supervisor through its complete
    /// shutdown report and inspect every retained task and cleanup outcome.
    pub fn supervisor(&self, budget: ShutdownBudget) -> Supervisor {
        Supervisor::with_process_capacity(budget, self.process_capacity)
    }

    /// Construct SQLx pool options without opening a connection.
    ///
    /// This probe escape hatch bypasses [`PreparedServing`]. A caller that uses
    /// it to create a live pool owns registration before return and awaited close.
    pub fn pool_options(&self) -> PgPoolOptions {
        self.pool.pool_options()
    }

    /// Construct native connection options at the explicit ambient-PG boundary.
    ///
    /// This probe escape hatch bypasses [`PreparedServing`]. Native formatting
    /// may expose credentials; do not log the returned value.
    pub fn connect_options_from_process(&self) -> Result<PgConnectOptions, SettingsError> {
        self.endpoint.connect_options_from_process()
    }

    /// Clone the already validated native worker configuration.
    /// Its native `Debug` representation exposes the configured worker identity.
    /// This probe escape hatch bypasses [`PreparedServing`]; a caller starting
    /// native work owns Batter registration and complete managed settlement.
    pub fn jobs_config(&self) -> JobsConfig {
        self.jobs.clone()
    }

    /// Clone the concrete production authenticator.
    /// The protected router path consumes it through [`PreparedHttp`] instead.
    pub fn authenticator(&self) -> BearerAuthenticator {
        self.authenticator.clone()
    }
}

impl fmt::Debug for ServingSettings {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ServingSettings([REDACTED])")
    }
}

impl fmt::Display for ServingSettings {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, formatter)
    }
}

/// Validated database-only inputs for offline maintenance.
///
/// Dedicated files and overrides reject HTTP, authentication, worker, and
/// lifecycle keys instead of parsing a serving-shaped superset and discarding it.
/// Captured environment may contain known serving keys, which are ignored without
/// validation; unknown reserved keys and every PG* key still fail closed.
pub struct MaintenanceSettings {
    endpoint: endpoint::MaintenanceEndpoint,
}

impl MaintenanceSettings {
    /// Capture process environment once, read only an explicitly selected file,
    /// then apply explicit database-only overrides.
    /// An environment `DATABASE_URL` has higher precedence than the selected file.
    pub fn from_process(
        file: Option<&Path>,
        overrides: SettingsSource,
    ) -> Result<Self, SettingsError> {
        let (file, environment) = process_sources(file)?;
        Self::from_sources(file, environment, overrides)
    }

    /// Load injected database-only sources without consulting process globals.
    pub fn from_sources(
        file: Option<SettingsSource>,
        environment: SettingsSource,
        overrides: SettingsSource,
    ) -> Result<Self, SettingsError> {
        let environment_names = serving_names();
        let values = merge_sources(
            MAINTENANCE_ROOT_NAMES,
            &environment_names,
            file,
            environment,
            overrides,
        )?;
        Ok(Self {
            endpoint: endpoint::MaintenanceEndpoint::parse(values.required("DATABASE_URL")?)?,
        })
    }

    /// Construct inert native database inputs at the explicit ambient-PG boundary.
    pub fn prepare(self) -> Result<PreparedMaintenance, SettingsError> {
        let connect_options = self.endpoint.connect_options_from_process()?;
        Ok(PreparedMaintenance { connect_options })
    }

    /// Construct native connection options at the explicit ambient-PG boundary.
    ///
    /// This probe escape hatch bypasses [`PreparedMaintenance`]. Native formatting
    /// may expose credentials; do not log the returned value.
    pub fn connect_options_from_process(&self) -> Result<PgConnectOptions, SettingsError> {
        self.endpoint.connect_options_from_process()
    }
}

impl fmt::Debug for MaintenanceSettings {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("MaintenanceSettings([REDACTED])")
    }
}

impl fmt::Display for MaintenanceSettings {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, formatter)
    }
}

/// Opaque serving-qualified HTTP inputs, consumed by [`crate::http::router`].
///
/// ```
/// use batter_example_reference_service::config::{PreparedHttp, ServingSettings};
///
/// fn prepare_http(settings: &ServingSettings) -> PreparedHttp {
///     settings.prepare_http()
/// }
/// ```
///
/// ```compile_fail
/// #![deny(unused_must_use)]
/// use batter_example_reference_service::config::ServingSettings;
///
/// fn cannot_discard(settings: &ServingSettings) {
///     settings.prepare_http();
/// }
/// ```
#[must_use = "prepared HTTP inputs must be transferred to the router"]
pub struct PreparedHttp {
    pub(crate) request_budget: ResponseConstructionBudget,
    pub(crate) bulkhead_capacity: BulkheadCapacity,
    pub(crate) authenticator: BearerAuthenticator,
}

impl fmt::Debug for PreparedHttp {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PreparedHttp([REDACTED])")
    }
}

/// Inert, non-cloneable ownership transfer into the serving runtime.
///
/// This value owns native constructor inputs but no socket, database connection,
/// migration, task, or worker. Only [`crate::runtime::run`] can consume the
/// complete serving preparation through the protected path.
///
/// ```
/// use batter::settings::SettingsError;
/// use batter_example_reference_service::config::{PreparedServing, ServingSettings};
///
/// fn prepare(
///     settings: ServingSettings,
/// ) -> Result<PreparedServing, SettingsError> {
///     batter_example_reference_service::runtime::prepare(settings)
/// }
/// ```
///
/// ```compile_fail,E0599
/// use batter_example_reference_service::config::PreparedServing;
///
/// fn cannot_duplicate(prepared: PreparedServing) {
///     let duplicate = prepared.clone();
/// }
/// ```
///
/// ```compile_fail
/// #![deny(unused_must_use)]
/// use batter::settings::SettingsError;
/// use batter_example_reference_service::{config::ServingSettings, runtime};
///
/// fn cannot_discard(settings: ServingSettings) -> Result<(), SettingsError> {
///     runtime::prepare(settings)?;
///     Ok(())
/// }
/// ```
#[must_use = "prepared serving inputs must be transferred to runtime::run"]
pub struct PreparedServing {
    bind: SocketAddr,
    pool_options: PgPoolOptions,
    connect_options: PgConnectOptions,
    supervisor: Supervisor,
    jobs: JobsConfig,
    http: PreparedHttp,
}

pub(crate) struct ServingParts {
    pub(crate) bind: SocketAddr,
    pub(crate) pool_options: PgPoolOptions,
    pub(crate) connect_options: PgConnectOptions,
    pub(crate) supervisor: Supervisor,
    pub(crate) jobs: JobsConfig,
    pub(crate) http: PreparedHttp,
}

impl PreparedServing {
    pub(crate) fn into_parts(self) -> ServingParts {
        ServingParts {
            bind: self.bind,
            pool_options: self.pool_options,
            connect_options: self.connect_options,
            supervisor: self.supervisor,
            jobs: self.jobs,
            http: self.http,
        }
    }
}

impl fmt::Debug for PreparedServing {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PreparedServing([REDACTED])")
    }
}

/// Inert, non-cloneable native inputs for an offline maintenance command.
///
/// This type has no conversion into [`PreparedServing`] or [`PreparedHttp`].
///
/// ```no_run
/// use batter::settings::SettingsSource;
/// use batter_example_reference_service::config::MaintenanceSettings;
///
/// let settings = MaintenanceSettings::from_sources(
///     None,
///     SettingsSource::default(),
///     SettingsSource::from_pairs([(
///         "DATABASE_URL".into(),
///         "postgres://operator@localhost/database?sslmode=disable".into(),
///     )])?,
/// )?;
/// let prepared = settings.prepare()?;
/// let _connect_options = prepared.into_connect_options();
/// # Ok::<(), batter::settings::SettingsError>(())
/// ```
///
/// ```compile_fail,E0599
/// use batter_example_reference_service::config::PreparedMaintenance;
///
/// fn cannot_duplicate(prepared: PreparedMaintenance) {
///     let duplicate = prepared.clone();
/// }
/// ```
///
/// ```compile_fail
/// #![deny(unused_must_use)]
/// use batter::settings::SettingsError;
/// use batter_example_reference_service::config::MaintenanceSettings;
///
/// fn cannot_discard(settings: MaintenanceSettings) -> Result<(), SettingsError> {
///     settings.prepare()?;
///     Ok(())
/// }
/// ```
#[must_use = "prepared maintenance inputs must be transferred to the offline command"]
pub struct PreparedMaintenance {
    connect_options: PgConnectOptions,
}

impl PreparedMaintenance {
    /// Consume the preparation into the native connection options.
    /// Native formatting may expose credentials; do not log this value.
    pub fn into_connect_options(self) -> PgConnectOptions {
        self.connect_options
    }
}

impl fmt::Debug for PreparedMaintenance {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PreparedMaintenance([REDACTED])")
    }
}
