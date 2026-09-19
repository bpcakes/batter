use std::time::Duration;
use tokio::time::Instant;

/// A rejected operational configuration, without configuration values/secrets.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConfigurationError {
    /// A strictly positive value is required.
    #[error("{0} must be greater than zero")]
    Zero(&'static str),
    /// A duration cannot be represented by the runtime clock.
    #[error("{0} is too large")]
    TooLarge(&'static str),
    /// The maximum backoff must not be smaller than its initial value.
    #[error("maximum backoff must be at least the initial backoff")]
    InvalidBackoff,
    /// A finalization reserve must leave positive time for the work phase.
    #[error("finalization reserve must leave positive work time")]
    InvalidReserve,
    /// Health observations must outlive delay, probe budget and scheduling margin.
    #[error("health maximum age must cover delay, probe budget and scheduling margin")]
    InvalidHealthTiming,
}

/// Stable, unique names make managed work identifiable without dynamic labels.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RegistrationError {
    /// Names must be 1–96 ASCII alphanumeric, dot, underscore, or hyphen bytes.
    #[error("invalid component name")]
    InvalidName,
    /// A name was already registered in this collection.
    #[error("component name is already registered: {0}")]
    Duplicate(&'static str),
}

pub(crate) fn name(value: &'static str) -> Result<(), RegistrationError> {
    if value.is_empty()
        || value.len() > 96
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
    {
        return Err(RegistrationError::InvalidName);
    }
    Ok(())
}

pub(crate) fn positive(
    value: Duration,
    field: &'static str,
) -> Result<Duration, ConfigurationError> {
    if value.is_zero() {
        return Err(ConfigurationError::Zero(field));
    }
    representable(value, field)?;
    Ok(value)
}

pub(crate) fn representable(
    value: Duration,
    field: &'static str,
) -> Result<(), ConfigurationError> {
    // Operational budgets longer than one year are almost certainly mistakes.
    if value > Duration::from_secs(365 * 24 * 60 * 60) {
        return Err(ConfigurationError::TooLarge(field));
    }
    Instant::now()
        .checked_add(value)
        .ok_or(ConfigurationError::TooLarge(field))?;
    Ok(())
}
