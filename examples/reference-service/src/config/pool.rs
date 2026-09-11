use super::MAX_DURATION;
use batter::settings::{SettingsError, SettingsSource, bounded_u64, milliseconds};
use sqlx::postgres::PgPoolOptions;
use std::{fmt, time::Duration};

pub(super) const NAMES: &[&str] = &[
    "BATTER_POOL_MAX_CONNECTIONS",
    "BATTER_POOL_MIN_CONNECTIONS",
    "BATTER_POOL_ACQUIRE_TIMEOUT_MS",
];

/// Validated application pool policy; native SQLx clamping is never validation.
#[derive(Clone)]
pub struct PoolSettings {
    max: u32,
    min: u32,
    acquire_timeout: Duration,
}
impl PoolSettings {
    /// Validate explicit native values before acquisition. Zero minimum is allowed.
    pub fn new(max: u32, min: u32, acquire_timeout: Duration) -> Result<Self, SettingsError> {
        if max == 0 {
            return Err(SettingsError::new(NAMES[0], "must be positive"));
        }
        if min > max {
            return Err(SettingsError::new(NAMES[1], "minimum exceeds maximum"));
        }
        if acquire_timeout.is_zero() || acquire_timeout > MAX_DURATION {
            return Err(SettingsError::new(NAMES[2], "outside permitted bounds"));
        }
        Ok(Self {
            max,
            min,
            acquire_timeout,
        })
    }
    /// Parse dedicated pool overrides over the reference defaults (8, 0, 3000 ms).
    pub fn from_source(source: &SettingsSource) -> Result<Self, SettingsError> {
        source.select(NAMES, &[], false)?;
        Self::from_values(source)
    }
    pub(super) fn from_values(source: &SettingsSource) -> Result<Self, SettingsError> {
        let max = bounded_u64(
            source.text(NAMES[0])?.unwrap_or("8"),
            NAMES[0],
            1,
            u32::MAX.into(),
        )?;
        let min = bounded_u64(
            source.text(NAMES[1])?.unwrap_or("0"),
            NAMES[1],
            0,
            u32::MAX.into(),
        )?;
        let timeout = milliseconds(
            source.text(NAMES[2])?.unwrap_or("3000"),
            NAMES[2],
            MAX_DURATION,
        )?;
        Self::new(
            u32::try_from(max)
                .map_err(|e| SettingsError::new(NAMES[0], "integer overflow").with_cause(e))?,
            u32::try_from(min)
                .map_err(|e| SettingsError::new(NAMES[1], "integer overflow").with_cause(e))?,
            timeout,
        )
    }
    /// Build native options used directly by `connect_with` or fixture ConnectionPlan.
    pub fn pool_options(&self) -> PgPoolOptions {
        PgPoolOptions::new()
            .max_connections(self.max)
            .min_connections(self.min)
            .acquire_timeout(self.acquire_timeout)
    }
}
impl fmt::Debug for PoolSettings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PoolSettings([REDACTED])")
    }
}
impl fmt::Display for PoolSettings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}
