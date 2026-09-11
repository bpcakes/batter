//! Application-owned authentication seam for the reference command.
//!
//! The production root maps one opaque bearer token to one configured owner.
//! Broader peer/proxy and request-metadata policy remains outside this task.
//!
//! ```
//! use axum::http::HeaderValue;
//! use batter::settings::SecretString;
//! use batter_example_reference_service::{
//!     auth::BearerAuthenticator,
//!     delivery::OwnerId,
//! };
//! use uuid::Uuid;
//!
//! let owner = OwnerId::new(Uuid::from_u128(1))?;
//! let auth = BearerAuthenticator::new(owner, SecretString::new("fake-example-token"))?;
//! let selected = auth.authenticate(Some(&HeaderValue::from_static(
//!     "Bearer fake-example-token",
//! )))?;
//! assert_eq!(selected, owner);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use crate::delivery::OwnerId;
use axum::http::HeaderValue;
use batter::settings::SecretString;

/// Maximum bearer token size accepted by the reference configuration.
pub const AUTH_TOKEN_MAX_BYTES: usize = 256;

/// Authentication configuration failure that never formats token contents.
#[derive(Debug, thiserror::Error)]
pub enum AuthConfigurationError {
    /// The configured token contains unsupported bytes or is empty.
    #[error("authentication token is invalid")]
    InvalidToken,
}

/// Request authentication failure. Wire mapping deliberately merges variants.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum AuthenticationError {
    /// No Authorization header was supplied.
    #[error("authentication is required")]
    Missing,
    /// The scheme or credential did not match.
    #[error("authentication failed")]
    Invalid,
}

/// Opaque bearer-token authenticator producing typed application authority.
#[derive(Clone)]
pub struct BearerAuthenticator {
    owner: OwnerId,
    token: SecretString,
}

impl BearerAuthenticator {
    /// Construct the production authentication seam from validated owner/token input.
    pub fn new(owner: OwnerId, token: SecretString) -> Result<Self, AuthConfigurationError> {
        let bytes = token.expose_secret().as_bytes();
        if bytes.is_empty()
            || bytes.len() > AUTH_TOKEN_MAX_BYTES
            || bytes.iter().any(|byte| !(0x21..=0x7e).contains(byte))
        {
            return Err(AuthConfigurationError::InvalidToken);
        }
        Ok(Self { owner, token })
    }

    /// Authenticate an HTTP Authorization header without trusting route/trace data.
    pub fn authenticate(
        &self,
        authorization: Option<&HeaderValue>,
    ) -> Result<OwnerId, AuthenticationError> {
        let authorization = authorization.ok_or(AuthenticationError::Missing)?;
        let supplied = authorization
            .to_str()
            .ok()
            .and_then(|value| value.strip_prefix("Bearer "))
            .ok_or(AuthenticationError::Invalid)?;
        if constant_time_eq(supplied.as_bytes(), self.token.expose_secret().as_bytes()) {
            Ok(self.owner)
        } else {
            Err(AuthenticationError::Invalid)
        }
    }
}

impl std::fmt::Debug for BearerAuthenticator {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("BearerAuthenticator([REDACTED])")
    }
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn authenticator() -> BearerAuthenticator {
        BearerAuthenticator::new(
            OwnerId::new(Uuid::from_u128(1)).unwrap(),
            SecretString::new("fake-auth-token"),
        )
        .unwrap()
    }

    #[test]
    fn only_the_exact_bearer_credential_selects_the_configured_owner() {
        let auth = authenticator();
        assert_eq!(
            auth.authenticate(Some(&HeaderValue::from_static("Bearer fake-auth-token")))
                .unwrap()
                .as_uuid(),
            Uuid::from_u128(1)
        );
        for header in [
            None,
            Some(HeaderValue::from_static("fake-auth-token")),
            Some(HeaderValue::from_static("Bearer wrong")),
        ] {
            assert!(auth.authenticate(header.as_ref()).is_err());
        }
        assert_eq!(format!("{auth:?}"), "BearerAuthenticator([REDACTED])");
    }
}
