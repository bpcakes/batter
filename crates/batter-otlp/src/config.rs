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
    /// Select positive durations representable at construction. If clock
    /// advancement later makes an extreme allowance unrepresentable, the
    /// adapter shortens it to a representable deadline. Attempts may exceed
    /// the interval; elapsed ticks then coalesce instead of accumulating work.
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
    let url = http_endpoint(value)?;
    if url.path() != "/v1/metrics" {
        return Err(SettingsError::new(
            "metrics.endpoint",
            "expected /v1/metrics",
        ));
    }
    Ok(url)
}

pub(crate) fn victoriametrics_endpoint(value: &str) -> Result<url::Url, SettingsError> {
    let url = http_endpoint(value)?;
    if !url.path().ends_with("/opentelemetry/v1/metrics")
        || url.path().split('/').skip(1).any(str::is_empty)
    {
        return Err(SettingsError::new(
            "metrics.endpoint",
            "expected a path ending in /opentelemetry/v1/metrics without empty segments",
        ));
    }
    Ok(url)
}

fn http_endpoint(value: &str) -> Result<url::Url, SettingsError> {
    let url = url::Url::parse(value)
        .map_err(|error| SettingsError::new("metrics.endpoint", "invalid URL").with_cause(error))?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(SettingsError::new(
            "metrics.endpoint",
            "expected HTTP(S) without credentials, query or fragment",
        ));
    }
    Ok(url)
}

pub(crate) fn identity(value: &str) -> Result<(), SettingsError> {
    bounded_identity(value, "metrics.service_name", 64)
}

pub(crate) fn instance_identity(value: &str) -> Result<(), SettingsError> {
    bounded_identity(value, "metrics.instance_id", 128)
}

fn bounded_identity(value: &str, field: &'static str, max: usize) -> Result<(), SettingsError> {
    if value.is_empty()
        || value.len() > max
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(SettingsError::new(
            field,
            "expected a nonempty bounded ASCII name",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn victoriametrics_profile_requires_its_ingestion_path_and_bounded_instance() {
        for value in [
            "http://localhost:8428/opentelemetry/v1/metrics",
            "https://collector.example/insert/12:34/opentelemetry/v1/metrics",
            "https://collector.example/proxy/opentelemetry/v1/metrics",
        ] {
            assert!(victoriametrics_endpoint(value).is_ok(), "{value}");
            assert!(endpoint(value).is_err(), "generic OTLP remains strict");
        }
        for value in [
            "http://localhost/v1/metrics",
            "http://localhost//opentelemetry/v1/metrics",
            "http://localhost/opentelemetry/v1/metrics/",
            "http://localhost/opentelemetry/v1/metrics?tenant=1",
            "http://localhost/opentelemetry/v1/metrics#fragment",
            "http://user:secret@localhost/opentelemetry/v1/metrics",
            "file:///opentelemetry/v1/metrics",
        ] {
            assert!(victoriametrics_endpoint(value).is_err(), "{value}");
        }
        for value in ["", "two words", "https://host", "秘密", &"a".repeat(129)] {
            assert!(
                instance_identity(value).is_err(),
                "invalid instance accepted"
            );
        }
        assert!(
            instance_identity(&"a".repeat(128)).is_ok(),
            "bounded ID accepted"
        );
    }
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
