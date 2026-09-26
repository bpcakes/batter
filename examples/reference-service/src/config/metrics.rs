//! Opt-in metrics collector settings.
//!
//! Absent `BATTER_METRICS_OTLP_ENDPOINT` keeps the default silent behavior. An
//! explicit value in a build without the `metrics-export` feature fails with a
//! fixed configuration error instead of silently disabling export. Enabled
//! export accepts only cleartext HTTP to a literal loopback address at
//! `/v1/metrics`, without credentials, query or fragment, and rejects ambient
//! `OTEL_*` configuration. No value is echoed in diagnostics.

use batter::settings::{SettingsError, SettingsSource};

pub(super) const NAMES: &[&str] = &["BATTER_METRICS_OTLP_ENDPOINT"];
const NAME: &str = "BATTER_METRICS_OTLP_ENDPOINT";
pub(super) const AMBIENT_PREFIXES: &[&str] = &["OTEL_"];

/// Validated collector selection, still inert.
pub(super) enum MetricsSettings {
    Disabled,
    #[cfg(feature = "metrics-export")]
    Otlp(url::Url),
}

/// Prepared diagnostic resources carried by the serving preparation.
pub(crate) enum PreparedMetrics {
    Disabled,
    #[cfg(feature = "metrics-export")]
    Otlp(Box<batter::otlp::Prepared>),
}

impl MetricsSettings {
    /// `ambient_otel` reports whether the captured environment contained any
    /// `OTEL_*` name; it matters only when export is enabled.
    pub(super) fn from_values(
        values: &SettingsSource,
        ambient_otel: bool,
    ) -> Result<Self, SettingsError> {
        let Some(endpoint) = values.text(NAME)? else {
            return Ok(Self::Disabled);
        };
        #[cfg(feature = "metrics-export")]
        {
            let endpoint = validate(endpoint)?;
            if ambient_otel {
                return Err(SettingsError::new(
                    "environment",
                    "ambient OpenTelemetry settings are unsupported",
                ));
            }
            Ok(Self::Otlp(endpoint))
        }
        #[cfg(not(feature = "metrics-export"))]
        {
            let _ = (endpoint, ambient_otel);
            Err(SettingsError::new(
                NAME,
                "metrics export is not compiled into this build",
            ))
        }
    }

    pub(super) fn prepare(self) -> Result<PreparedMetrics, SettingsError> {
        match self {
            Self::Disabled => Ok(PreparedMetrics::Disabled),
            #[cfg(feature = "metrics-export")]
            Self::Otlp(endpoint) => batter::otlp::prepare(
                endpoint.as_str(),
                "batter-example-reference-service",
                batter::otlp::Schedule::new(
                    std::time::Duration::from_secs(10),
                    std::time::Duration::from_secs(3),
                    std::time::Duration::from_secs(5),
                )?,
            )
            .map(|prepared| PreparedMetrics::Otlp(Box::new(prepared))),
        }
    }
}

#[cfg(feature = "metrics-export")]
fn validate(value: &str) -> Result<url::Url, SettingsError> {
    use url::Host;
    let url = url::Url::parse(value)
        .map_err(|error| SettingsError::new(NAME, "invalid URL").with_cause(error))?;
    if url.scheme() != "http" {
        return Err(SettingsError::new(NAME, "collector URL must use http"));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(SettingsError::new(NAME, "URL credentials are forbidden"));
    }
    if url.query().is_some() || url.fragment().is_some() || url.path() != "/v1/metrics" {
        return Err(SettingsError::new(
            NAME,
            "URL must name the /v1/metrics path only",
        ));
    }
    let loopback = match url.host() {
        Some(Host::Ipv4(address)) => address.is_loopback(),
        Some(Host::Ipv6(address)) => address.is_loopback(),
        Some(Host::Domain(_)) | None => false,
    };
    if !loopback {
        return Err(SettingsError::new(
            NAME,
            "collector host must be a literal loopback address",
        ));
    }
    Ok(url)
}

impl batter::service::Diagnostics for PreparedMetrics {
    type Report = crate::diagnostics::MetricsExport;

    fn install(
        self,
        completion: batter::service::DiagnosticCompletion,
    ) -> impl Future<Output = Self::Report> + Send + 'static {
        let future: std::pin::Pin<Box<dyn Future<Output = Self::Report> + Send>> = match self {
            Self::Disabled => Box::pin(async move {
                let _ = completion.wait().await;
                crate::diagnostics::MetricsExport::Disabled
            }),
            #[cfg(feature = "metrics-export")]
            Self::Otlp(prepared) => Box::pin((*prepared).install(completion)),
        };
        future
    }
}
