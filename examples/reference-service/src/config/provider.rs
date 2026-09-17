use crate::provider::ProviderClient;
use batter::{
    admission::{Bulkhead, BulkheadCapacity},
    settings::{SecretString, SettingsError, SettingsSource},
};
use std::fmt;
use url::{Host, Url};

pub(super) const NAMES: &[&str] = &[
    "BATTER_PROVIDER_BASE_URL",
    "BATTER_PROVIDER_TOKEN",
    "BATTER_PROVIDER_CAPACITY",
];

/// Validated, inert inputs for the selected reference effect protocol.
pub(super) struct ProviderSettings {
    base_url: Url,
    token: SecretString,
    capacity: BulkheadCapacity,
}

impl ProviderSettings {
    pub(super) fn from_values(values: &SettingsSource) -> Result<Self, SettingsError> {
        let base_url = Url::parse(values.required(NAMES[0])?)
            .map_err(|error| SettingsError::new(NAMES[0], "invalid URL").with_cause(error))?;
        validate_base_url(&base_url)?;

        let token = values.required(NAMES[1])?;
        if token.is_empty()
            || token.len() > 256
            || token.bytes().any(|byte| !(0x21..=0x7e).contains(&byte))
        {
            return Err(SettingsError::new(NAMES[1], "invalid token"));
        }

        Ok(Self {
            base_url,
            token: SecretString::new(token.to_owned()),
            capacity: BulkheadCapacity::new(super::capacity(values, NAMES[2])?).map_err(
                |error| SettingsError::new(NAMES[2], "native validation failed").with_cause(error),
            )?,
        })
    }

    pub(super) fn prepare(self) -> Result<(ProviderClient, Bulkhead), SettingsError> {
        let client = ProviderClient::new(self.base_url, self.token).map_err(|error| {
            SettingsError::new("provider", "native client construction failed").with_cause(error)
        })?;
        Ok((client, Bulkhead::new(self.capacity)))
    }
}

fn validate_base_url(url: &Url) -> Result<(), SettingsError> {
    let secure = match url.scheme() {
        "https" => true,
        "http" => false,
        _ => return Err(SettingsError::new(NAMES[0], "unsupported URL scheme")),
    };
    if !url.username().is_empty() || url.password().is_some() {
        return Err(SettingsError::new(
            NAMES[0],
            "URL credentials are forbidden",
        ));
    }
    if url.query().is_some() || url.fragment().is_some() || url.path() != "/" {
        return Err(SettingsError::new(NAMES[0], "URL must name one origin"));
    }
    let host = url
        .host()
        .ok_or_else(|| SettingsError::new(NAMES[0], "URL host is required"))?;
    if !secure && !loopback(host) {
        return Err(SettingsError::new(
            NAMES[0],
            "cleartext URL must use loopback",
        ));
    }
    Ok(())
}

fn loopback(host: Host<&str>) -> bool {
    match host {
        Host::Domain(_) => false,
        Host::Ipv4(address) => address.is_loopback(),
        Host::Ipv6(address) => address.is_loopback(),
    }
}

impl fmt::Debug for ProviderSettings {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ProviderSettings([REDACTED])")
    }
}
