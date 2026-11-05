use sqlx::types::Uuid;
use std::fmt;

/// Exact PostgreSQL cluster and database identity supplied by the maintenance owner.
///
/// ```
/// use batter_example_reference_service::retirement::DatabaseIdentity;
/// let identity = DatabaseIdentity::new(-42, 17).unwrap();
/// assert_eq!(identity.system_identifier(), -42);
/// assert_eq!(identity.database_oid(), 17);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DatabaseIdentity {
    pub(super) system_identifier: i64,
    pub(super) database_oid: u32,
}

impl DatabaseIdentity {
    /// PostgreSQL cluster identifier, preserving its signed bigint representation.
    pub fn system_identifier(self) -> i64 {
        self.system_identifier
    }

    /// Database OID within this exact cluster.
    pub fn database_oid(self) -> u32 {
        self.database_oid
    }

    /// Validate nonzero cluster and database identifiers before starting work.
    /// PostgreSQL exposes its unsigned cluster identifier through a signed bigint.
    pub fn new(system_identifier: i64, database_oid: u32) -> Result<Self, RetirementError> {
        if system_identifier == 0 || database_oid == 0 {
            return Err(RetirementError::InvalidIdentity);
        }
        Ok(Self {
            system_identifier,
            database_oid,
        })
    }
}

/// Observed legacy definition state after native admission was disabled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DefinitionState {
    /// No definition exists. This is not a durable disabled tombstone.
    Absent,
    /// Existing native definition is disabled; additive catalog sync preserves it.
    Disabled,
}

/// Completed database observations; does not assert deployment replacement.
#[derive(Debug)]
pub struct RetirementReport {
    /// Identity verified before any mutation.
    pub identity: DatabaseIdentity,
    /// Native admission state, also checked after cancellation.
    pub definition: DefinitionState,
    /// Jobs cancelled by this invocation, excluding previously terminal history.
    pub cancelled: u64,
}

/// Original native cancellation error, published before any optional readback.
pub struct CancellationFailure {
    /// Exact global legacy job whose cancellation was not acknowledged.
    pub job_id: Uuid,
    /// Earlier cancellations acknowledged by the native API in this invocation.
    pub cancelled_before_failure: u64,
    /// Original native cause; a commit failure leaves the durable outcome unknown.
    pub native: runledger_postgres::Error,
}

impl fmt::Debug for CancellationFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("native cancellation failed; original cause retained; readback not attempted")
    }
}

impl fmt::Display for CancellationFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

impl std::error::Error for CancellationFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.native)
    }
}

/// Explicit refusal or failure; formatting never includes native error contents.
#[derive(thiserror::Error)]
pub enum RetirementError {
    /// The expected database identity was zero, so no target can match it.
    #[error("nonzero expected database identity required")]
    InvalidIdentity,
    /// The maintenance session reached a different database.
    #[error("maintenance target identity does not match")]
    WrongDatabase {
        /// Identity the maintenance session actually observed.
        actual: DatabaseIdentity,
    },
    /// Other clients or prepared transactions remained after cancellation.
    #[error("database is not quiescent for offline retirement")]
    NotOffline {
        /// Target-database backends outside the maintenance session.
        other_backends: i64,
        /// Prepared transactions still held in the target database.
        prepared_transactions: i64,
        /// Jobs this invocation had already cancelled.
        cancelled: u64,
    },
    /// The legacy definition was enabled again before verification finished.
    #[error("legacy definition was enabled during retirement")]
    DefinitionEnabled,
    /// The pool replaced its single maintenance session during the command.
    #[error("maintenance session replacement refused")]
    SessionReplaced {
        /// The command's own failure, when it failed before the replacement was seen.
        #[source]
        original: Option<Box<RetirementError>>,
    },
    /// The retirement command could not be registered with its process.
    #[error("command registration failed")]
    Registration(#[from] batter::RegistrationError),
    /// A maintenance query failed.
    #[error("retirement database operation failed")]
    Database(#[from] sqlx::Error),
    /// The native package could not disable the legacy definition.
    #[error("native definition disable failed")]
    Native(#[from] runledger_postgres::Error),
    /// Native cancellation of one job failed; its original error is retained.
    #[error("native cancellation failed; original cause retained; readback not attempted")]
    Cancellation(#[source] Box<CancellationFailure>),
}

impl fmt::Debug for RetirementError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
