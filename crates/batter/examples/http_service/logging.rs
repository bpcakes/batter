use batter::settings::RedactedError;
use std::env::VarError;
use tracing_subscriber::{EnvFilter, filter::FromEnvError};

const DEFAULT_FILTER: &str = "batter=info,http_service=info";

// Source traversal is deliberate; normal formatting uses shared static metadata.
pub(super) type LogConfigurationError = RedactedError<FromEnvError>;

pub(super) fn filter(value: Result<String, VarError>) -> Result<EnvFilter, LogConfigurationError> {
    let parsed = match value {
        Ok(value) => EnvFilter::try_new(value).map_err(FromEnvError::from),
        Err(VarError::NotPresent) => return Ok(EnvFilter::new(DEFAULT_FILTER)),
        Err(error) => Err(FromEnvError::from(error)),
    };
    parsed.map_err(|cause| RedactedError::new("RUST_LOG", "invalid configuration", cause))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error;
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};
    use tracing_subscriber::filter::ParseError;

    #[test]
    fn only_absent_configuration_uses_the_default() {
        let default = filter(Err(VarError::NotPresent)).unwrap();
        assert_eq!(
            default.to_string(),
            EnvFilter::new(DEFAULT_FILTER).to_string()
        );
        let configured = filter(Ok("info,batter=warn".into())).unwrap();
        assert_eq!(
            configured.to_string(),
            EnvFilter::new("info,batter=warn").to_string()
        );
        assert_eq!(filter(Ok("off".into())).unwrap().to_string(), "off");
    }

    #[test]
    fn invalid_filter_is_rejected_with_a_retained_source() {
        let error = filter(Ok("batter=invalid-level".into())).unwrap_err();
        assert!(error.source().unwrap().source().unwrap().is::<ParseError>());
        assert_eq!(format!("{error:?}"), "RUST_LOG: invalid configuration");
        assert_eq!(error.to_string(), "RUST_LOG: invalid configuration");
    }

    #[test]
    fn non_unicode_configuration_is_retained_but_not_printed() {
        let value = OsString::from_vec(b"private-filter-marker\xff".to_vec());
        let error = filter(Err(VarError::NotUnicode(value.clone()))).unwrap_err();
        let original = error
            .source()
            .unwrap()
            .source()
            .unwrap()
            .downcast_ref::<VarError>()
            .unwrap();
        assert_eq!(original, &VarError::NotUnicode(value));
        assert_eq!(format!("{error:?}"), "RUST_LOG: invalid configuration");
        assert_eq!(error.to_string(), "RUST_LOG: invalid configuration");
    }
}
