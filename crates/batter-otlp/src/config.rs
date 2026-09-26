use batter_core::settings::SettingsError;
use std::time::Duration;

/// Validated periodic and final export allowances.
/// These bound yielding asynchronous I/O, not synchronous SDK collection or
/// closure. An in-flight periodic attempt settles before the final allowance
/// starts. Missed interval ticks coalesce; there is no request queue or retry.
#[derive(Clone, Copy, Debug)]
pub struct Schedule {
    pub(crate) interval: Duration,
    pub(crate) attempt: Duration,
    pub(crate) final_allowance: Duration,
}

impl Schedule {
    /// Select positive, representable durations. Attempts may exceed the
    /// interval; elapsed ticks then coalesce instead of accumulating work.
    pub fn new(
        interval: Duration,
        attempt: Duration,
        final_allowance: Duration,
    ) -> Result<Self, SettingsError> {
        for value in [interval, attempt, final_allowance] {
            if value.is_zero() || tokio::time::Instant::now().checked_add(value).is_none() {
                return Err(SettingsError::new(
                    "metrics.schedule",
                    "durations must be positive and representable",
                ));
            }
        }
        Ok(Self {
            interval,
            attempt,
            final_allowance,
        })
    }
}

pub(crate) fn endpoint(value: &str) -> Result<url::Url, SettingsError> {
    let url = url::Url::parse(value)
        .map_err(|error| SettingsError::new("metrics.endpoint", "invalid URL").with_cause(error))?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/v1/metrics"
    {
        return Err(SettingsError::new(
            "metrics.endpoint",
            "expected HTTP(S) /v1/metrics without credentials, query or fragment",
        ));
    }
    Ok(url)
}

pub(crate) fn identity(value: &str) -> Result<(), SettingsError> {
    if value.is_empty()
        || value.len() > 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(SettingsError::new(
            "metrics.service_name",
            "expected 1 to 64 ASCII name bytes",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn configuration_rejects_unbounded_or_ambiguous_inputs() {
        for duration in [Duration::ZERO, Duration::MAX] {
            assert!(
                Schedule::new(duration, Duration::from_secs(1), Duration::from_secs(1)).is_err()
            );
            assert!(
                Schedule::new(Duration::from_secs(1), duration, Duration::from_secs(1)).is_err()
            );
            assert!(
                Schedule::new(Duration::from_secs(1), Duration::from_secs(1), duration).is_err()
            );
        }
        for endpoint_value in [
            "file:///v1/metrics",
            "http://user:secret@localhost/v1/metrics",
            "http://localhost/wrong",
            "http://localhost/v1/metrics?secret=1",
        ] {
            assert!(endpoint(endpoint_value).is_err());
        }
        assert!(endpoint("https://collector.example/v1/metrics").is_ok());
        assert!(identity("").is_err());
        assert!(identity(&"a".repeat(65)).is_err());
        assert!(identity("example-service").is_ok());
    }
}
