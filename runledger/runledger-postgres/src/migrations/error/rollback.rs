use std::fmt;

use super::SchemaCompatibilityError;

/// Both guarded index repair and its explicit rollback failed.
///
/// The original refusal or SQL failure and native rollback failure are retained
/// independently. This does not prove rollback or remote connection termination.
/// Default formatting redacts both; explicit accessors are for trusted diagnostics.
///
/// ```rust,no_run
/// # fn inspect(error: &runledger_postgres::SchemaCompatibilityError) {
/// if let runledger_postgres::SchemaCompatibilityError::OnlineIndexRollback(pair) = error {
///     let original = pair.operation_error();
///     let rollback = pair.rollback_error();
///     // Retain both causes; neither authorizes automatic replay.
/// }
/// # }
/// ```
pub struct OnlineIndexRollbackError {
    operation: Box<SchemaCompatibilityError>,
    rollback: sqlx::Error,
}

impl OnlineIndexRollbackError {
    pub(in crate::migrations) fn new(
        operation: SchemaCompatibilityError,
        rollback: sqlx::Error,
    ) -> Self {
        Self {
            operation: Box::new(operation),
            rollback,
        }
    }

    /// Original repair error or refusal, for trusted inspection.
    #[must_use]
    pub fn operation_error(&self) -> &SchemaCompatibilityError {
        &self.operation
    }

    /// Independent rollback error; it does not establish transaction outcome.
    #[must_use]
    pub fn rollback_error(&self) -> &sqlx::Error {
        &self.rollback
    }
}

impl fmt::Display for OnlineIndexRollbackError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("online index repair and rollback both failed")
    }
}
impl fmt::Debug for OnlineIndexRollbackError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
impl std::error::Error for OnlineIndexRollbackError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&*self.operation)
    }
}
