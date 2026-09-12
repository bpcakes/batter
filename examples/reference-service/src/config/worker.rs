use super::MAX_DURATION;
use batter::settings::{SecretString, SettingsError, SettingsSource, bounded_u64, milliseconds};
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

/// Reference worker settings. Setup can omit identity but cannot construct a worker.
/// Polling uses native validation and does not participate in an execution witness.
#[derive(Clone)]
pub struct WorkerSettings {
    id: Option<SecretString>,
    poll: Duration,
    batch: i64,
    lease: i32,
    concurrency: usize,
    reaper: Duration,
    schedule: Duration,
    retry: i32,
}
impl WorkerSettings {
    /// Parse dedicated worker overrides with explicit required identity and defaults.
    /// This does not read environment variables or spawn workers.
    pub fn from_source(source: &SettingsSource) -> Result<Self, SettingsError> {
        source.select(NAMES, &[], false)?;
        Self::from_values(source, true)
    }
    pub(super) fn from_values(
        source: &SettingsSource,
        required: bool,
    ) -> Result<Self, SettingsError> {
        let text = |i, default| -> Result<&str, SettingsError> {
            Ok(source.text(NAMES[i])?.unwrap_or(default))
        };
        let integer = |i, default, max| bounded_u64(text(i, default)?, NAMES[i], 1, max);
        let id = source.text(NAMES[0])?.map(SecretString::new);
        let settings = Self {
            id,
            poll: milliseconds(text(1, "500")?, NAMES[1], MAX_DURATION)?,
            batch: i64::try_from(integer(2, "16", JOBS_CLAIM_BATCH_SIZE_MAX as u64)?)
                .map_err(|e| SettingsError::new(NAMES[2], "integer overflow").with_cause(e))?,
            lease: i32::try_from(integer(3, "60", i32::MAX as u64)?)
                .map_err(|e| SettingsError::new(NAMES[3], "integer overflow").with_cause(e))?,
            concurrency: usize::try_from(integer(
                4,
                "32",
                tokio::sync::Semaphore::MAX_PERMITS as u64,
            )?)
            .map_err(|e| SettingsError::new(NAMES[4], "integer overflow").with_cause(e))?,
            reaper: Duration::from_secs(integer(5, "15", MAX_DURATION.as_secs())?),
            schedule: Duration::from_secs(integer(6, "30", MAX_DURATION.as_secs())?),
            retry: i32::try_from(integer(7, "30000", i32::MAX as u64)?)
                .map_err(|e| SettingsError::new(NAMES[7], "integer overflow").with_cause(e))?,
        };
        if required || settings.id.is_some() {
            settings.jobs_config()?;
        }
        Ok(settings)
    }
    /// Produce all eight explicit native fields and run native validation.
    /// The returned native type's Debug exposes its worker identity.
    /// The composition root prepares the native supervisor with this input and
    /// transfers it to [`batter_runledger::register`] before starting work.
    pub fn jobs_config(&self) -> Result<JobsConfig, SettingsError> {
        let id = self
            .id
            .as_ref()
            .ok_or_else(|| SettingsError::new(NAMES[0], "missing value"))?;
        let config = JobsConfig {
            worker_id: id.expose_secret().to_owned(),
            poll_interval: self.poll,
            claim_batch_size: self.batch,
            lease_ttl_seconds: self.lease,
            max_global_concurrency: self.concurrency,
            reaper_interval: self.reaper,
            schedule_poll_interval: self.schedule,
            reaper_retry_delay_ms: self.retry,
        };
        config
            .validate()
            .map_err(|e| SettingsError::new("worker", "native validation failed").with_cause(e))?;
        Ok(config)
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
