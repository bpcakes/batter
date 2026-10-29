use std::{fmt, time::Duration};

use super::SchemaCompatibilityError;

/// The profile setting that bounded a migration-lock wait.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum MigrationTimeoutSetting {
    /// `statement_timeout`.
    Statement,
    /// `lock_timeout`.
    Lock,
    /// `transaction_timeout`.
    Transaction,
}

impl MigrationTimeoutSetting {
    /// PostgreSQL's setting name, for diagnostics rather than error parsing.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Statement => "statement_timeout",
            Self::Lock => "lock_timeout",
            Self::Transaction => "transaction_timeout",
        }
    }
}

/// A known local lock-wait expiry, not a connection fault or replay permission.
///
/// ```rust,no_run
/// # async fn demo(db: &runledger_postgres::RunledgerDatabase) {
/// if let Err(runledger_postgres::SchemaCompatibilityError::MigrationLockTimeout(expiry)) =
///     runledger_postgres::migrate_after_idempotency_cutover(db).await
/// {
///     let setting = expiry.setting();
///     let bound = expiry.bound();
///     // Application startup policy decides what to do with this observation.
/// }
/// # }
/// ```
#[derive(Debug)]
pub struct MigrationLockTimeout {
    setting: MigrationTimeoutSetting,
    bound: Duration,
}

impl MigrationLockTimeout {
    pub(in crate::migrations) fn new(setting: MigrationTimeoutSetting, bound: Duration) -> Self {
        Self { setting, bound }
    }

    /// Setting selected by the smallest nonzero profile bound.
    #[must_use]
    pub fn setting(&self) -> MigrationTimeoutSetting {
        self.setting
    }

    /// The positive duration allowed for lock acquisition.
    #[must_use]
    pub fn bound(&self) -> Duration {
        self.bound
    }
}

impl fmt::Display for MigrationLockTimeout {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "migration lock wait exceeded {} ({} ms)",
            self.setting.name(),
            self.bound.as_millis()
        )
    }
}

impl std::error::Error for MigrationLockTimeout {}

/// The failed phase of library-owned online index execution.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum OnlineIndexPhase {
    /// Reading the catalog before deciding whether a build is needed.
    Inspect,
    /// Establishing the build session's settings, before index SQL runs.
    Configure,
    /// Dropping an interrupted index, building it or checking its result.
    Build,
    /// Restoring settings after a successful build and catalog check.
    Restore,
}

/// Retained online-index execution context with redacted default diagnostics.
///
/// A recorded version means its history was observed or its application was
/// acknowledged before this attempt. It does not prove the current index state,
/// undo effects, or authorize replay. `None` identifies external preparation,
/// which has not recorded the index's migration. Restoration failure after a
/// successful build is distinct from a build failure; a second restoration
/// failure is retained alongside the primary cause.
///
/// ```rust,no_run
/// # async fn demo(db: &runledger_postgres::RunledgerDatabase) {
/// if let Err(runledger_postgres::SchemaCompatibilityError::OnlineIndexExecution(failure)) =
///     runledger_postgres::migrate_after_idempotency_cutover(db).await
/// {
///     let index = failure.index();
///     let recorded = failure.recorded_version();
///     match failure.phase() {
///         runledger_postgres::OnlineIndexPhase::Inspect => {},
///         runledger_postgres::OnlineIndexPhase::Configure => {},
///         runledger_postgres::OnlineIndexPhase::Build => {},
///         runledger_postgres::OnlineIndexPhase::Restore => {},
///         _ => {},
///     }
///     // Inspect failure.cause() only in trusted diagnostics.
/// }
/// # }
/// ```
pub struct OnlineIndexExecutionError {
    index: &'static str,
    recorded_version: Option<i64>,
    phase: OnlineIndexPhase,
    cause: Box<SchemaCompatibilityError>,
    restoration_error: Option<sqlx::Error>,
}

impl OnlineIndexExecutionError {
    pub(in crate::migrations) fn new(
        index: &'static str,
        recorded_version: Option<i64>,
        phase: OnlineIndexPhase,
        cause: SchemaCompatibilityError,
        restoration_error: Option<sqlx::Error>,
    ) -> Self {
        Self {
            index,
            recorded_version,
            phase,
            cause: Box::new(cause),
            restoration_error,
        }
    }

    /// The bundled index whose execution failed.
    #[must_use]
    pub fn index(&self) -> &'static str {
        self.index
    }

    /// Previously observed or acknowledged migration version, if recorded.
    #[must_use]
    pub fn recorded_version(&self) -> Option<i64> {
        self.recorded_version
    }

    /// Distinguish failed build execution from failed session restoration.
    #[must_use]
    pub fn phase(&self) -> OnlineIndexPhase {
        self.phase
    }

    /// Original cause for explicit, trusted inspection; may contain SQL/data.
    #[must_use]
    pub fn cause(&self) -> &SchemaCompatibilityError {
        &self.cause
    }

    /// A restoration failure returned in addition to the primary build failure.
    #[must_use]
    pub fn restoration_error(&self) -> Option<&sqlx::Error> {
        self.restoration_error.as_ref()
    }
}

impl fmt::Display for OnlineIndexExecutionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "online index {} failed during {:?}",
            self.index, self.phase
        )?;
        if let Some(version) = self.recorded_version {
            write!(f, "; migration {version} was recorded")?;
        }
        Ok(())
    }
}

impl fmt::Debug for OnlineIndexExecutionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl std::error::Error for OnlineIndexExecutionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&*self.cause)
    }
}
