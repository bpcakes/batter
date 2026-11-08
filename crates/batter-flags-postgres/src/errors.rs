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
/// A store failure. Every class but [`ErrorClass::CommitUnknown`] is certain:
/// a failure before COMMIT, or a database rejection of COMMIT, so nothing was
/// written. `CommitUnknown` may have committed. All retained causes are
/// available through `source()`; formatting is redacted.
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
    /// The caller interrupted an audited mutation before COMMIT was sent; it
    /// was rolled back and nothing was written.
    Interrupted,
    /// An unaudited operation (heartbeat, stop or prune) may have committed:
    /// its commit, including the native checks just before COMMIT, failed
    /// without a database rejection or ran out of time. The next heartbeat
    /// restates presence, and a prune that did commit deleted only rows that
    /// were already due; an unknown stop is its own case,
    /// [`StopFailure::Unknown`](crate::StopFailure::Unknown). Audited writes
    /// never use this class: theirs is `WriteOutcome::Uncertain`.
    CommitUnknown,
}
pub(crate) enum ErrorKind {
    Class(ErrorClass),
    /// The installed revision, 0 when no flags migration has run.
    SchemaBehind(i32),
    Sql(SqlxFailure),
    Transaction(PgTransactionError),
    Unconfirmed(PgTransactionError),
    Scope(Box<PgScopeFailure<StoreError>>),
    Snapshot(Box<PgSnapshotError<StoreError>>),
    Other(Box<dyn Error + Send + Sync>),
}
impl StoreError {
    /// A bounded diagnostic category; it never grants retry authority.
    pub fn class(&self) -> ErrorClass {
        match &self.0 {
            ErrorKind::Class(c) => *c,
            ErrorKind::SchemaBehind(_) => ErrorClass::SchemaRevision,
            ErrorKind::Unconfirmed(_) => ErrorClass::CommitUnknown,
            // An interrupted mutation whose recovery or rollback also failed
            // still wrote nothing: the transaction never reached COMMIT and
            // its connection is retired.
            ErrorKind::Scope(failure)
                if matches!(
                    failure.as_ref(),
                    batter_sqlx::PgScopeFailure::Recovery { application, .. }
                        if application.class() == ErrorClass::Interrupted
                ) =>
            {
                ErrorClass::Interrupted
            }
            // An interrupted mutation whose rollback also failed still wrote
            // nothing: the transaction never reached COMMIT and its
            // connection is retired.
            ErrorKind::Other(other)
                if other
                    .downcast_ref::<crate::RollbackFailure>()
                    .is_some_and(|failure| {
                        failure.operation().class() == ErrorClass::Interrupted
                    }) =>
            {
                ErrorClass::Interrupted
            }
            _ => ErrorClass::Database,
        }
    }
    /// For [`ErrorClass::SchemaRevision`] from a load, the installed schema
    /// revision: 0 when no flags migration has run.
    ///
    /// The observation survives a failed cleanup: a load that found the
    /// schema behind and then could not roll back still reports it.
    pub fn installed_schema_revision(&self) -> Option<i32> {
        match &self.0 {
            ErrorKind::SchemaBehind(found) => Some(*found),
            ErrorKind::Snapshot(snapshot) => match snapshot.as_ref() {
                PgSnapshotError::Inspection(inspection)
                | PgSnapshotError::Cleanup { inspection, .. } => {
                    inspection.installed_schema_revision()
                }
                PgSnapshotError::Transaction(_) => None,
            },
            ErrorKind::Other(other) => other
                .downcast_ref::<crate::RollbackFailure>()
                .and_then(|failure| failure.operation().installed_schema_revision()),
            ErrorKind::Class(_)
            | ErrorKind::Sql(_)
            | ErrorKind::Transaction(_)
            | ErrorKind::Unconfirmed(_)
            | ErrorKind::Scope(_) => None,
        }
    }
    /// The SQLSTATE of the database error behind this failure, if one
    /// caused it: a code, never a message or detail.
    pub fn sqlstate(&self) -> Option<String> {
        let mut source: Option<&(dyn Error + 'static)> = Some(self);
        while let Some(error) = source {
            if let Some(sqlx::Error::Database(database)) = error.downcast_ref::<sqlx::Error>()
                && let Some(code) = database.code()
            {
                return Some(code.into_owned());
            }
            source = error.source();
        }
        None
    }

    pub(crate) fn schema_behind(found: i32) -> Self {
        Self(ErrorKind::SchemaBehind(found))
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
            ErrorKind::Class(_) | ErrorKind::SchemaBehind(_) => None,
            ErrorKind::Sql(e) => Some(e),
            ErrorKind::Transaction(e) | ErrorKind::Unconfirmed(e) => Some(e),
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
