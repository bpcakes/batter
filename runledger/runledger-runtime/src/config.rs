use std::time::Duration;

use thiserror::Error;

mod environment;
pub use environment::JobsEnvironmentError;

/// Maximum batch size accepted by runtime worker, intent promoter, scheduler,
/// and reaper loops.
pub const JOBS_CLAIM_BATCH_SIZE_MAX: i64 = 1_000;

#[derive(Debug, Clone)]
pub struct JobsConfig {
    pub worker_id: String,
    pub poll_interval: Duration,
    pub claim_batch_size: i64,
    pub lease_ttl_seconds: i32,
    pub max_global_concurrency: usize,
    pub reaper_interval: Duration,
    pub schedule_poll_interval: Duration,
    pub reaper_retry_delay_ms: i32,
}

/// Polling and batch controls for durable enqueue-intent promotion.
///
/// [`crate::Supervisor`] derives these values from [`JobsConfig`] by default so
/// existing deployments keep their current behavior. Use this type when intent
/// traffic needs a cadence independent from ordinary queue claiming.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct IntentPromoterConfig {
    poll_interval: Duration,
    batch_size: i64,
}

impl IntentPromoterConfig {
    #[must_use]
    pub const fn new(poll_interval: Duration, batch_size: i64) -> Self {
        Self {
            poll_interval,
            batch_size,
        }
    }

    #[must_use]
    pub const fn from_jobs_config(config: &JobsConfig) -> Self {
        Self::new(config.poll_interval, config.claim_batch_size)
    }

    pub fn validate(&self) -> Result<(), JobsConfigValidationError> {
        if self.poll_interval.is_zero() {
            return Err(JobsConfigValidationError::ZeroPollInterval);
        }
        validate_claim_batch_size(self.batch_size)
    }

    #[must_use]
    pub const fn poll_interval(&self) -> Duration {
        self.poll_interval
    }

    #[must_use]
    pub const fn batch_size(&self) -> i64 {
        self.batch_size
    }
}

#[non_exhaustive]
#[derive(Debug, Clone, Copy, Error, Eq, PartialEq)]
pub enum JobsConfigValidationError {
    #[error("jobs config worker_id must not be empty")]
    EmptyWorkerId,
    #[error("jobs config poll_interval must be greater than zero")]
    ZeroPollInterval,
    #[error("jobs config claim_batch_size must be between 1 and 1000, got {actual}")]
    InvalidClaimBatchSize { actual: i64 },
    #[error("jobs config lease_ttl_seconds must be at least 1, got {actual}")]
    InvalidLeaseTtlSeconds { actual: i32 },
    #[error("jobs config max_global_concurrency must be at least 1")]
    InvalidMaxGlobalConcurrency,
    #[error("jobs config reaper_interval must be greater than zero")]
    ZeroReaperInterval,
    #[error("jobs config schedule_poll_interval must be greater than zero")]
    ZeroSchedulePollInterval,
    #[error("jobs config reaper_retry_delay_ms must be at least 1, got {actual}")]
    InvalidReaperRetryDelayMs { actual: i32 },
}

impl JobsConfig {
    pub fn validate(&self) -> Result<(), JobsConfigValidationError> {
        if self.worker_id.trim().is_empty() {
            return Err(JobsConfigValidationError::EmptyWorkerId);
        }
        if self.poll_interval.is_zero() {
            return Err(JobsConfigValidationError::ZeroPollInterval);
        }
        validate_claim_batch_size(self.claim_batch_size)?;
        if self.lease_ttl_seconds < 1 {
            return Err(JobsConfigValidationError::InvalidLeaseTtlSeconds {
                actual: self.lease_ttl_seconds,
            });
        }
        if self.max_global_concurrency < 1 {
            return Err(JobsConfigValidationError::InvalidMaxGlobalConcurrency);
        }
        if self.reaper_interval.is_zero() {
            return Err(JobsConfigValidationError::ZeroReaperInterval);
        }
        if self.schedule_poll_interval.is_zero() {
            return Err(JobsConfigValidationError::ZeroSchedulePollInterval);
        }
        if self.reaper_retry_delay_ms < 1 {
            return Err(JobsConfigValidationError::InvalidReaperRetryDelayMs {
                actual: self.reaper_retry_delay_ms,
            });
        }

        Ok(())
    }

    pub(crate) fn validate_worker_loop(&self) -> Result<(), JobsConfigValidationError> {
        if self.worker_id.trim().is_empty() {
            return Err(JobsConfigValidationError::EmptyWorkerId);
        }
        if self.poll_interval.is_zero() {
            return Err(JobsConfigValidationError::ZeroPollInterval);
        }
        validate_claim_batch_size(self.claim_batch_size)?;
        if self.lease_ttl_seconds < 1 {
            return Err(JobsConfigValidationError::InvalidLeaseTtlSeconds {
                actual: self.lease_ttl_seconds,
            });
        }
        if self.max_global_concurrency < 1 {
            return Err(JobsConfigValidationError::InvalidMaxGlobalConcurrency);
        }

        Ok(())
    }

    pub(crate) fn validate_scheduler_loop(&self) -> Result<(), JobsConfigValidationError> {
        validate_claim_batch_size(self.claim_batch_size)?;
        if self.schedule_poll_interval.is_zero() {
            return Err(JobsConfigValidationError::ZeroSchedulePollInterval);
        }

        Ok(())
    }

    pub(crate) fn validate_reaper_loop(&self) -> Result<(), JobsConfigValidationError> {
        validate_claim_batch_size(self.claim_batch_size)?;
        if self.reaper_interval.is_zero() {
            return Err(JobsConfigValidationError::ZeroReaperInterval);
        }
        if self.reaper_retry_delay_ms < 1 {
            return Err(JobsConfigValidationError::InvalidReaperRetryDelayMs {
                actual: self.reaper_retry_delay_ms,
            });
        }

        Ok(())
    }
}

fn validate_claim_batch_size(claim_batch_size: i64) -> Result<(), JobsConfigValidationError> {
    if (1..=JOBS_CLAIM_BATCH_SIZE_MAX).contains(&claim_batch_size) {
        return Ok(());
    }

    Err(JobsConfigValidationError::InvalidClaimBatchSize {
        actual: claim_batch_size,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> JobsConfig {
        JobsConfig {
            worker_id: "config-test-worker".to_string(),
            poll_interval: Duration::from_millis(1),
            claim_batch_size: 1,
            lease_ttl_seconds: 1,
            max_global_concurrency: 1,
            reaper_interval: Duration::from_millis(1),
            schedule_poll_interval: Duration::from_millis(1),
            reaper_retry_delay_ms: 1,
        }
    }

    #[test]
    fn validate_accepts_minimum_direct_config_values() {
        test_config()
            .validate()
            .expect("minimum direct config should be valid");
    }

    #[test]
    fn intent_promoter_config_validates_independently() {
        let jobs_config = test_config();
        assert_eq!(
            IntentPromoterConfig::from_jobs_config(&jobs_config),
            IntentPromoterConfig::new(jobs_config.poll_interval, jobs_config.claim_batch_size)
        );
        assert_eq!(
            IntentPromoterConfig::new(Duration::ZERO, 1).validate(),
            Err(JobsConfigValidationError::ZeroPollInterval)
        );
        assert_eq!(
            IntentPromoterConfig::new(Duration::from_millis(1), 0).validate(),
            Err(JobsConfigValidationError::InvalidClaimBatchSize { actual: 0 })
        );
    }

    #[test]
    fn validate_rejects_invalid_direct_config_values() {
        let cases = [
            {
                let mut config = test_config();
                config.worker_id = "  ".to_string();
                (config, JobsConfigValidationError::EmptyWorkerId)
            },
            {
                let mut config = test_config();
                config.poll_interval = Duration::ZERO;
                (config, JobsConfigValidationError::ZeroPollInterval)
            },
            {
                let mut config = test_config();
                config.claim_batch_size = 0;
                (
                    config,
                    JobsConfigValidationError::InvalidClaimBatchSize { actual: 0 },
                )
            },
            {
                let mut config = test_config();
                config.claim_batch_size = JOBS_CLAIM_BATCH_SIZE_MAX + 1;
                (
                    config,
                    JobsConfigValidationError::InvalidClaimBatchSize {
                        actual: JOBS_CLAIM_BATCH_SIZE_MAX + 1,
                    },
                )
            },
            {
                let mut config = test_config();
                config.lease_ttl_seconds = 0;
                (
                    config,
                    JobsConfigValidationError::InvalidLeaseTtlSeconds { actual: 0 },
                )
            },
            {
                let mut config = test_config();
                config.max_global_concurrency = 0;
                (
                    config,
                    JobsConfigValidationError::InvalidMaxGlobalConcurrency,
                )
            },
            {
                let mut config = test_config();
                config.reaper_interval = Duration::ZERO;
                (config, JobsConfigValidationError::ZeroReaperInterval)
            },
            {
                let mut config = test_config();
                config.schedule_poll_interval = Duration::ZERO;
                (config, JobsConfigValidationError::ZeroSchedulePollInterval)
            },
            {
                let mut config = test_config();
                config.reaper_retry_delay_ms = 0;
                (
                    config,
                    JobsConfigValidationError::InvalidReaperRetryDelayMs { actual: 0 },
                )
            },
        ];

        for (config, expected) in cases {
            assert_eq!(config.validate(), Err(expected));
        }
    }
}
