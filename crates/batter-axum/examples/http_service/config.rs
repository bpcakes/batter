use batter::{
    admission::Bulkhead,
    settings::{SettingsError, SettingsSource, bounded_u64, milliseconds, read_file},
};
use std::{ffi::OsString, net::SocketAddr, time::Duration};

const NAMES: &[&str] = &[
    "BATTER_BIND",
    "BATTER_REQUEST_TIMEOUT_MS",
    "BATTER_BULKHEAD_CAPACITY",
    "RUST_LOG",
];

// Application names, defaults and precedence live at this root.
pub(super) struct Config {
    pub bind: SocketAddr,
    pub request_budget: Duration,
    pub bulkhead_capacity: usize,
    pub log_filter: Option<String>,
}
impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Config([REDACTED])")
    }
}
impl Config {
    pub fn load() -> Result<Self, SettingsError> {
        let environment = SettingsSource::from_pairs(std::env::vars_os())?;
        let file = environment
            .get("BATTER_ENV_FILE")
            .map(|path| read_file(path, 64 * 1024))
            .transpose()?;
        Self::from_sources(file, environment, SettingsSource::default())
    }
    pub fn from_sources(
        file: Option<SettingsSource>,
        environment: SettingsSource,
        overrides: SettingsSource,
    ) -> Result<Self, SettingsError> {
        let mut values = SettingsSource::from_pairs([
            (OsString::from("BATTER_BIND"), "127.0.0.1:3000".into()),
            ("BATTER_REQUEST_TIMEOUT_MS".into(), "2000".into()),
            ("BATTER_BULKHEAD_CAPACITY".into(), "32".into()),
        ])?;
        if let Some(file) = file {
            values.overlay(file.select(NAMES, &[], false)?);
        }
        let mut env_names = NAMES.to_vec();
        env_names.push("BATTER_ENV_FILE");
        values.overlay(environment.select(&env_names, &["BATTER_"], true)?);
        values.overlay(overrides.select(NAMES, &[], false)?);
        let bind = values
            .required("BATTER_BIND")?
            .parse()
            .map_err(|e| SettingsError::new("BATTER_BIND", "invalid address").with_cause(e))?;
        let request_budget = milliseconds(
            values.required("BATTER_REQUEST_TIMEOUT_MS")?,
            "BATTER_REQUEST_TIMEOUT_MS",
            Duration::from_secs(365 * 24 * 60 * 60),
        )?;
        // Validate the actual native duration boundary before acquiring resources.
        batter::operation::OperationContext::new(request_budget).map_err(|e| {
            SettingsError::new("BATTER_REQUEST_TIMEOUT_MS", "invalid duration").with_cause(e)
        })?;
        let bulkhead_capacity = bounded_u64(
            values.required("BATTER_BULKHEAD_CAPACITY")?,
            "BATTER_BULKHEAD_CAPACITY",
            1,
            tokio::sync::Semaphore::MAX_PERMITS as u64,
        )? as usize;
        Bulkhead::new(bulkhead_capacity).map_err(|e| {
            SettingsError::new("BATTER_BULKHEAD_CAPACITY", "invalid capacity").with_cause(e)
        })?;
        Ok(Self {
            bind,
            request_budget,
            bulkhead_capacity,
            log_filter: values.text("RUST_LOG")?.map(str::to_owned),
        })
    }
}
