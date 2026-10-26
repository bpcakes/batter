use std::fmt;

use sqlx::migrate::MigrateError;

use super::SchemaCompatibilityError;

/// Both an owned migration/preparation operation and its lock release failed.
///
/// The original operation error retains any index phase, recorded version and
/// session-restoration failure. The unlock error is independent of it. Neither
/// failure authorizes replay or proves remote session termination; connection
/// retirement remains armed. Direct `Debug` and `Display` redact both causes.
/// Accessors and `Error::source()` (the operation error) are for trusted
/// inspection: chain-walking reporters can disclose native SQL/error details.
///
/// ```rust,no_run
/// # async fn demo(db: &runledger_postgres::RunledgerDatabase) {
/// if let Err(runledger_postgres::SchemaCompatibilityError::MigrationCleanup(failure)) =
///     runledger_postgres::migrate_after_idempotency_cutover(db).await
/// {
///     let operation = failure.operation_error();
///     let unlock = failure.unlock_error();
///     // Inspect both only in application-selected trusted diagnostics.
/// }
/// # }
/// ```
pub struct MigrationCleanupError {
    operation: Box<SchemaCompatibilityError>,
    unlock: MigrateError,
}

impl MigrationCleanupError {
    pub(in crate::migrations) fn new(
        operation: SchemaCompatibilityError,
        unlock: MigrateError,
    ) -> Self {
        Self {
            operation: Box::new(operation),
            unlock,
        }
    }

    /// Original operation failure, including any retained online-index context.
    /// May contain SQL/data; inspect only in trusted diagnostics.
    #[must_use]
    pub fn operation_error(&self) -> &SchemaCompatibilityError {
        &self.operation
    }

    /// Independent advisory-lock release failure; may contain native details.
    #[must_use]
    pub fn unlock_error(&self) -> &MigrateError {
        &self.unlock
    }
}

impl fmt::Display for MigrationCleanupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Runledger migration operation and advisory-lock release both failed")
    }
}

impl fmt::Debug for MigrationCleanupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl std::error::Error for MigrationCleanupError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&*self.operation)
    }
}
