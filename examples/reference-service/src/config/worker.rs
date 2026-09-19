use super::MAX_DURATION;
use batter::settings::{SettingsError, SettingsSource, bounded_u64, milliseconds};
use runledger_runtime::config::{JOBS_CLAIM_BATCH_SIZE_MAX, JobsConfig};
use std::{fmt, time::Duration};

pub(super) const NAMES: &[&str] = &[
    "JOBS_WORKER_ID",
    "JOBS_POLL_INTERVAL_MS",
    "JOBS_CLAIM_BATCH_SIZE",
    "JOBS_LEASE_TTL_SECONDS",
    "JOBS_MAX_GLOBAL_CONCURRENCY",
    "JOBS_REAPER_INTERVAL_SECONDS",
    "JOBS_SCHEDULE_POLL_INTERVAL_SECONDS",
    "JOBS_REAPER_RETRY_DELAY_MS",
];

/// Validated reference worker settings.
///
/// The identity is mandatory here. Offline maintenance never parses this type.
/// Polling uses native validation and does not participate in an execution witness.
#[derive(Clone)]
pub struct WorkerSettings(JobsConfig);

impl WorkerSettings {
    /// Parse dedicated worker overrides with explicit required identity and defaults.
    /// This does not read environment variables or spawn workers.
    pub fn from_source(source: &SettingsSource) -> Result<Self, SettingsError> {
        source.select(NAMES, &[], false)?;
        Self::from_values(source)
    }
    pub(super) fn from_values(source: &SettingsSource) -> Result<Self, SettingsError> {
        let text = |i, default| -> Result<&str, SettingsError> {
            Ok(source.text(NAMES[i])?.unwrap_or(default))
        };
        let integer = |i, default, max| bounded_u64(text(i, default)?, NAMES[i], 1, max);
        let config = JobsConfig {
            worker_id: source.required(NAMES[0])?.to_owned(),
            poll_interval: milliseconds(text(1, "500")?, NAMES[1], MAX_DURATION)?,
            claim_batch_size: i64::try_from(integer(2, "16", JOBS_CLAIM_BATCH_SIZE_MAX as u64)?)
                .map_err(|e| SettingsError::new(NAMES[2], "integer overflow").with_cause(e))?,
            lease_ttl_seconds: i32::try_from(integer(3, "60", i32::MAX as u64)?)
                .map_err(|e| SettingsError::new(NAMES[3], "integer overflow").with_cause(e))?,
            max_global_concurrency: usize::try_from(integer(
                4,
                "32",
                tokio::sync::Semaphore::MAX_PERMITS as u64,
            )?)
            .map_err(|e| SettingsError::new(NAMES[4], "integer overflow").with_cause(e))?,
            reaper_interval: Duration::from_secs(integer(5, "15", MAX_DURATION.as_secs())?),
            schedule_poll_interval: Duration::from_secs(integer(6, "30", MAX_DURATION.as_secs())?),
            reaper_retry_delay_ms: i32::try_from(integer(7, "30000", i32::MAX as u64)?)
                .map_err(|e| SettingsError::new(NAMES[7], "integer overflow").with_cause(e))?,
        };
        config
            .validate()
            .map_err(|e| SettingsError::new("worker", "native validation failed").with_cause(e))?;
        Ok(Self(config))
    }
    /// Clone all eight native fields after validation during construction.
    /// The returned native type's Debug exposes its worker identity.
    /// This section-level probe escape hatch does not register or own native work.
    /// A direct caller must transfer prepared work through
    /// [`batter::runledger::register`] and observe its complete managed settlement.
    /// The protected service path uses `PreparedServing` instead.
    pub fn jobs_config(&self) -> JobsConfig {
        self.0.clone()
    }

    pub(super) fn into_jobs_config(self) -> JobsConfig {
        self.0
    }
}
impl fmt::Debug for WorkerSettings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("WorkerSettings([REDACTED])")
    }
}
impl fmt::Display for WorkerSettings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}
