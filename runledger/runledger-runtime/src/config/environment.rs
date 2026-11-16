use std::{env::VarError, str::FromStr, time::Duration};

use super::{IntentPromoterConfig, JOBS_CLAIM_BATCH_SIZE_MAX, JobsConfig};

/// A rejected environment setting. No value or parser error is retained.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[error("invalid jobs environment setting: {setting}")]
pub struct JobsEnvironmentError {
    setting: &'static str,
}

impl JobsEnvironmentError {
    /// Name of the invalid setting, never its supplied value.
    pub const fn setting(&self) -> &'static str {
        self.setting
    }
}

type Lookup<'a> = &'a mut dyn FnMut(&str) -> Result<String, VarError>;

impl JobsConfig {
    /// Read worker settings, defaulting only when a variable is absent.
    /// Invalid Unicode, malformed numbers, blank worker IDs and values outside
    /// the environment bounds are errors. Lease TTL must be at least 10 seconds
    /// and reaper retry delay at least 1000 ms; other numeric minima are one.
    /// Claim batches cannot exceed [`JOBS_CLAIM_BATCH_SIZE_MAX`].
    ///
    /// Migration: `JobsConfig::from_env()` now returns `Result`; use
    /// `let config = JobsConfig::from_env()?;`. No infallible shim is retained
    /// because it would restore silent replacement of invalid configuration.
    ///
    /// ```no_run
    /// # fn example() -> Result<(), runledger_runtime::config::JobsEnvironmentError> {
    /// let config = runledger_runtime::config::JobsConfig::from_env()?;
    /// assert!(config.max_global_concurrency > 0);
    /// # Ok(()) }
    /// ```
    pub fn from_env() -> Result<Self, JobsEnvironmentError> {
        Self::from_lookup(&mut |name| std::env::var(name))
    }

    fn from_lookup(lookup: Lookup<'_>) -> Result<Self, JobsEnvironmentError> {
        let worker_id = read(lookup, "JOBS_WORKER_ID")?
            .unwrap_or_else(|| format!("worker-{}", uuid::Uuid::now_v7()));
        require(!worker_id.trim().is_empty(), "JOBS_WORKER_ID")?;
        Ok(Self {
            worker_id,
            poll_interval: Duration::from_millis(number(
                lookup,
                "JOBS_POLL_INTERVAL_MS",
                500,
                1,
                u64::MAX,
            )?),
            claim_batch_size: number(
                lookup,
                "JOBS_CLAIM_BATCH_SIZE",
                16,
                1,
                JOBS_CLAIM_BATCH_SIZE_MAX,
            )?,
            lease_ttl_seconds: number(lookup, "JOBS_LEASE_TTL_SECONDS", 60, 10, i32::MAX)?,
            max_global_concurrency: number(
                lookup,
                "JOBS_MAX_GLOBAL_CONCURRENCY",
                32,
                1,
                usize::MAX,
            )?,
            reaper_interval: Duration::from_secs(number(
                lookup,
                "JOBS_REAPER_INTERVAL_SECONDS",
                15,
                1,
                u64::MAX,
            )?),
            schedule_poll_interval: Duration::from_secs(number(
                lookup,
                "JOBS_SCHEDULE_POLL_INTERVAL_SECONDS",
                30,
                1,
                u64::MAX,
            )?),
            reaper_retry_delay_ms: number(
                lookup,
                "JOBS_REAPER_RETRY_DELAY_MS",
                30_000,
                1_000,
                i32::MAX,
            )?,
        })
    }
}

impl IntentPromoterConfig {
    /// Read promoter settings, defaulting only absent variables to 500 ms and 16.
    /// Polling must be nonzero and batches must be between one and 1000.
    ///
    /// Migration: `IntentPromoterConfig::from_env()` now returns `Result`; use
    /// `let config = IntentPromoterConfig::from_env()?;`. No infallible shim
    /// preserves the invalid-as-absent behavior.
    ///
    /// ```no_run
    /// # fn example() -> Result<(), runledger_runtime::config::JobsEnvironmentError> {
    /// let config = runledger_runtime::config::IntentPromoterConfig::from_env()?;
    /// # let _ = config;
    /// # Ok(()) }
    /// ```
    pub fn from_env() -> Result<Self, JobsEnvironmentError> {
        Self::from_lookup(Duration::from_millis(500), 16, &mut |name| {
            std::env::var(name)
        })
    }

    /// Read promoter settings, inheriting worker cadence only for absent variables.
    /// Invalid supplied or inherited values are rejected, never clamped.
    ///
    /// Migration: `IntentPromoterConfig::from_env_with_jobs_config_defaults` now
    /// returns `Result`; append `?` to the call. No infallible shim is retained
    /// because it would restore invalid-as-absent configuration.
    ///
    /// ```no_run
    /// # fn example() -> Result<(), runledger_runtime::config::JobsEnvironmentError> {
    /// use runledger_runtime::config::{JobsConfig, IntentPromoterConfig};
    /// let jobs = JobsConfig::from_env()?;
    /// let promoter = IntentPromoterConfig::from_env_with_jobs_config_defaults(&jobs)?;
    /// # let _ = promoter;
    /// # Ok(()) }
    /// ```
    pub fn from_env_with_jobs_config_defaults(
        config: &JobsConfig,
    ) -> Result<Self, JobsEnvironmentError> {
        Self::from_lookup(config.poll_interval, config.claim_batch_size, &mut |name| {
            std::env::var(name)
        })
    }

    fn from_lookup(
        poll_default: Duration,
        batch_default: i64,
        lookup: Lookup<'_>,
    ) -> Result<Self, JobsEnvironmentError> {
        let poll_interval = match read(lookup, "JOBS_INTENT_PROMOTER_POLL_INTERVAL_MS")? {
            Some(value) => {
                Duration::from_millis(parse(&value, "JOBS_INTENT_PROMOTER_POLL_INTERVAL_MS")?)
            }
            None => poll_default,
        };
        require(
            !poll_interval.is_zero(),
            "JOBS_INTENT_PROMOTER_POLL_INTERVAL_MS",
        )?;
        let batch_size = number(
            lookup,
            "JOBS_INTENT_PROMOTER_BATCH_SIZE",
            batch_default,
            1,
            JOBS_CLAIM_BATCH_SIZE_MAX,
        )?;
        Ok(Self::new(poll_interval, batch_size))
    }
}

fn read(lookup: Lookup<'_>, setting: &'static str) -> Result<Option<String>, JobsEnvironmentError> {
    match lookup(setting) {
        Ok(value) => Ok(Some(value)),
        Err(VarError::NotPresent) => Ok(None),
        Err(VarError::NotUnicode(_)) => Err(JobsEnvironmentError { setting }),
    }
}

fn parse<T: FromStr>(value: &str, setting: &'static str) -> Result<T, JobsEnvironmentError> {
    value.parse().map_err(|_| JobsEnvironmentError { setting })
}

fn number<T: FromStr + PartialOrd>(
    lookup: Lookup<'_>,
    setting: &'static str,
    default: T,
    min: T,
    max: T,
) -> Result<T, JobsEnvironmentError> {
    let value = match read(lookup, setting)? {
        Some(value) => parse(&value, setting)?,
        None => default,
    };
    require(value >= min && value <= max, setting)?;
    Ok(value)
}

fn require(valid: bool, setting: &'static str) -> Result<(), JobsEnvironmentError> {
    if valid {
        Ok(())
    } else {
        Err(JobsEnvironmentError { setting })
    }
}

#[cfg(test)]
mod tests;
