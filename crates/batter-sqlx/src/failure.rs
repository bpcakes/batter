use std::{error::Error, fmt};

/// Diagnostic categories only; none authorizes replay or determines commit state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum FailureClass {
    /// Pool closed or acquisition timed out. Not an operation deadline.
    PoolUnavailable,
    /// I/O, TLS or protocol failure; external effects may have occurred.
    Transport,
    /// A native database response; inspect the cause for application policy.
    Database,
    /// Configuration, decoding, missing data or another native error.
    Other,
}

/// Original SQLx cause with fixed default diagnostics.
///
/// Debug and Display omit native contents, including SQL, credentials and server
/// messages. `source`, [`Self::native`] and [`Self::into_native`] deliberately
/// expose the original error for trusted inspection. Error-chain reporters and
/// SQLx's own tracing are application-owned and can disclose those contents.
/// Operation interruption remains a separate `OperationError::Interrupted`;
/// neither this wrapper nor its classification authorizes automatic retries.
pub struct SqlxFailure(sqlx::Error);

impl SqlxFailure {
    /// Inspect the original cause in a trusted context.
    pub fn native(&self) -> &sqlx::Error {
        &self.0
    }

    /// Recover the original native error without string conversion.
    pub fn into_native(self) -> sqlx::Error {
        self.0
    }

    /// Conservatively categorize the native variant, never its error text.
    pub fn class(&self) -> FailureClass {
        match self.0 {
            sqlx::Error::PoolClosed | sqlx::Error::PoolTimedOut => FailureClass::PoolUnavailable,
            sqlx::Error::Io(_) | sqlx::Error::Tls(_) | sqlx::Error::Protocol(_) => {
                FailureClass::Transport
            }
            sqlx::Error::Database(_) => FailureClass::Database,
            _ => FailureClass::Other,
        }
    }
}

impl From<sqlx::Error> for SqlxFailure {
    fn from(error: sqlx::Error) -> Self {
        Self(error)
    }
}

impl fmt::Display for SqlxFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PostgreSQL operation failed")
    }
}

impl fmt::Debug for SqlxFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl Error for SqlxFailure {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.0)
    }
}
