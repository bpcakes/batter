use batter_sqlx::{PgScopeFailure, PgSnapshotError, PgTransactionError, SqlxFailure};
use std::{error::Error, fmt};

/// Invalid construction input. Formatting never echoes input.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct ValidationError;
impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("invalid flags store configuration or request")
    }
}
impl Error for ValidationError {}
/// A certain failure before commit, or a database rejection of COMMIT.
/// All retained causes are available through `source()`; formatting is redacted.
pub struct StoreError(pub(crate) ErrorKind);
/// Stable failure category, without database messages or request contents.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum ErrorClass {
    /// PostgreSQL older than 18.
    UnsupportedServer,
    /// Loading a standby cannot refresh authority.
    Standby,
    /// Installed flags schema is too old.
    SchemaRevision,
    /// A total operation deadline expired before commit.
    Deadline,
    /// Invalid persisted data.
    InvalidData,
    /// SQLx, acquisition, migration or boundary failure.
    Database,
}
pub(crate) enum ErrorKind {
    Class(ErrorClass),
    Sql(SqlxFailure),
    Transaction(PgTransactionError),
    Scope(Box<PgScopeFailure<StoreError>>),
    Snapshot(Box<PgSnapshotError<StoreError>>),
    Other(Box<dyn Error + Send + Sync>),
}
impl StoreError {
    /// A bounded diagnostic category; it never grants retry authority.
    pub fn class(&self) -> ErrorClass {
        match &self.0 {
            ErrorKind::Class(c) => *c,
            _ => ErrorClass::Database,
        }
    }
    pub(crate) fn classified(class: ErrorClass) -> Self {
        Self(ErrorKind::Class(class))
    }
}
impl From<sqlx::Error> for StoreError {
    fn from(e: sqlx::Error) -> Self {
        Self(ErrorKind::Sql(e.into()))
    }
}
impl From<PgTransactionError> for StoreError {
    fn from(e: PgTransactionError) -> Self {
        Self(ErrorKind::Transaction(e))
    }
}
impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "flags store failed ({:?})", self.class())
    }
}
impl fmt::Debug for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
impl Error for StoreError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match &self.0 {
            ErrorKind::Class(_) => None,
            ErrorKind::Sql(e) => Some(e),
            ErrorKind::Transaction(e) => Some(e),
            ErrorKind::Scope(e) => Some(e.as_ref()),
            ErrorKind::Snapshot(e) => Some(e.as_ref()),
            ErrorKind::Other(e) => Some(e.as_ref()),
        }
    }
}
pub(crate) fn check_server(version: i32) -> Result<(), StoreError> {
    if version < 180_000 {
        Err(StoreError::classified(ErrorClass::UnsupportedServer))
    } else {
        Ok(())
    }
}
#[allow(
    clippy::fn_params_excessive_bools,
    reason = "private injected PostgreSQL pg_is_in_recovery scalar, not a public policy flag"
)]
pub(crate) fn check_primary(recovery: bool) -> Result<(), StoreError> {
    if recovery {
        Err(StoreError::classified(ErrorClass::Standby))
    } else {
        Ok(())
    }
}
