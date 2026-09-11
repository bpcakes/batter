//! Application-owned source policy and validated native constructor inputs.
//!
//! Defaults < selected literal file < captured environment < explicit overrides.
//! Dedicated sources reject unknown keys. Environment ignores unrelated names,
//! rejects unknown BATTER_/JOBS_ names and all PG* names (including empty values).
//! Selected files are limited to 64 KiB; no file is discovered implicitly.
//!
//! Serve requires DATABASE_URL with explicit password and JOBS_WORKER_ID. Setup
//! permits passwordless TCP endpoints and missing worker identity. Both require
//! explicit sslmode. See the package README for the supported URL subset.
//! Loading performs no acquisition, binding or task spawning. Native SQLx option
//! construction additionally checks the real environment; callers must keep that
//! environment unchanged. Native options and explicitly inspected error causes
//! are outside redacted formatting guarantees.
//!
//! ```
//! use batter::settings::SettingsSource;
//! use batter_example_reference_service::config::{ConfigMode, RootSettings};
//! let overrides = SettingsSource::from_pairs([
//!     ("DATABASE_URL".into(), "postgres://user:fake@localhost/db?sslmode=disable".into()),
//!     ("JOBS_WORKER_ID".into(), "worker-example".into()),
//! ])?;
//! let config = RootSettings::from_sources(ConfigMode::Serve, None,
//!     SettingsSource::default(), overrides)?;
//! assert_eq!(config.pool_options().get_max_connections(), 8);
//! # Ok::<(), batter::settings::SettingsError>(())
//! ```

mod endpoint;
mod pool;
mod worker;
pub use pool::PoolSettings;
pub use worker::WorkerSettings;

use batter::{
    admission::Bulkhead,
    lifecycle::{ShutdownBudget, ShutdownHandle, Supervisor},
    settings::{SettingsError, SettingsSource, bounded_u64, milliseconds, read_file},
};
use batter_axum::RequestPolicy;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use std::{fmt, net::SocketAddr, path::Path, time::Duration};

const MAX_DURATION: Duration = Duration::from_secs(365 * 24 * 60 * 60);
const ROOT_NAMES: &[&str] = &[
    "BATTER_BIND",
    "BATTER_REQUEST_TIMEOUT_MS",
    "BATTER_BULKHEAD_CAPACITY",
    "BATTER_PROCESS_CAPACITY",
    "DATABASE_URL",
];

/// Root purpose controls required credentials and whether worker identity is required.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfigMode {
    /// Serving requires explicit nonempty password and worker identity.
    Serve,
    /// Operator setup may use externally provisioned local trust authentication.
    Setup,
}

/// Validated root inputs. Names and policy belong to this example, not Batter.
#[derive(Clone)]
pub struct RootSettings {
    bind: SocketAddr,
    request_budget: Duration,
    bulkhead_capacity: usize,
    process_capacity: usize,
    pool: PoolSettings,
    worker: WorkerSettings,
    endpoint: endpoint::Endpoint,
}
impl RootSettings {
    /// Capture process environment once, read only an explicitly selected file,
    /// then apply explicit overrides. Selected file failures never fall back.
    pub fn from_process(
        mode: ConfigMode,
        file: Option<&Path>,
        overrides: SettingsSource,
    ) -> Result<Self, SettingsError> {
        let environment = SettingsSource::from_pairs(std::env::vars_os())?;
        let file = file.map(|path| read_file(path, 64 * 1024)).transpose()?;
        Self::from_sources(mode, file, environment, overrides)
    }
    /// Load injected sources without consulting process globals or constructing resources.
    /// Sources are structurally validated before parsing winning values.
    pub fn from_sources(
        mode: ConfigMode,
        file: Option<SettingsSource>,
        environment: SettingsSource,
        overrides: SettingsSource,
    ) -> Result<Self, SettingsError> {
        let names: Vec<_> = ROOT_NAMES
            .iter()
            .chain(pool::NAMES)
            .chain(worker::NAMES)
            .copied()
            .collect();
        let mut values = SettingsSource::default();
        if let Some(file) = file {
            values.overlay(file.select(&names, &[], false)?);
        }
        values.overlay(environment.select(&names, &["BATTER_", "JOBS_", "PG"], true)?);
        values.overlay(overrides.select(&names, &[], false)?);
        let bind = values
            .text("BATTER_BIND")?
            .unwrap_or("127.0.0.1:3000")
            .parse()
            .map_err(|e| SettingsError::new("BATTER_BIND", "invalid address").with_cause(e))?;
        let request_budget = milliseconds(
            values.text("BATTER_REQUEST_TIMEOUT_MS")?.unwrap_or("2000"),
            "BATTER_REQUEST_TIMEOUT_MS",
            MAX_DURATION,
        )?;
        RequestPolicy::new(ShutdownHandle::new(), request_budget).map_err(|e| {
            SettingsError::new("BATTER_REQUEST_TIMEOUT_MS", "native validation failed")
                .with_cause(e)
        })?;
        let capacity = |name| -> Result<usize, SettingsError> {
            usize::try_from(bounded_u64(
                values.text(name)?.unwrap_or("32"),
                name,
                1,
                tokio::sync::Semaphore::MAX_PERMITS as u64,
            )?)
            .map_err(|e| SettingsError::new(name, "integer overflow").with_cause(e))
        };
        Ok(Self {
            bind,
            request_budget,
            bulkhead_capacity: capacity("BATTER_BULKHEAD_CAPACITY")?,
            process_capacity: capacity("BATTER_PROCESS_CAPACITY")?,
            pool: PoolSettings::from_values(&values)?,
            worker: WorkerSettings::from_values(&values, mode == ConfigMode::Serve)?,
            endpoint: endpoint::Endpoint::parse(values.required("DATABASE_URL")?, mode)?,
        })
    }
    /// Native bind address; binding remains an owned startup action.
    pub fn bind(&self) -> SocketAddr {
        self.bind
    }
    /// Construct the actual HTTP response-construction policy.
    pub fn request_policy(
        &self,
        handle: ShutdownHandle,
    ) -> Result<RequestPolicy, batter::ConfigurationError> {
        RequestPolicy::new(handle, self.request_budget)
    }
    /// Construct SQLx pool options, without opening a connection.
    pub fn pool_options(&self) -> PgPoolOptions {
        self.pool.pool_options()
    }
    /// Construct native connection options using the current process environment.
    /// SQLx reads ambient defaults even without passfile lookup, so this effectful
    /// boundary is separate from deterministic `from_sources` parsing.
    /// Rejects any current PG* environment setting. Keep environment unchanged
    /// during native construction. Raw Debug and URL conversion expose credentials.
    pub fn connect_options_from_process(&self) -> Result<PgConnectOptions, SettingsError> {
        self.endpoint.connect_options_from_process()
    }
    /// Construct a separately bounded finite-process supervisor; does not start it.
    pub fn supervisor(
        &self,
        budget: ShutdownBudget,
    ) -> Result<Supervisor, batter::ConfigurationError> {
        Supervisor::with_process_capacity(budget, self.process_capacity)
    }
    /// Construct process-local operation admission, distinct from finite task capacity.
    pub fn bulkhead(&self) -> Result<Bulkhead, batter::ConfigurationError> {
        Bulkhead::new(self.bulkhead_capacity)
    }
    /// Borrow validated worker settings for the explicit native builder.
    pub fn worker(&self) -> &WorkerSettings {
        &self.worker
    }
}
impl fmt::Debug for RootSettings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RootSettings([REDACTED])")
    }
}
impl fmt::Display for RootSettings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}
