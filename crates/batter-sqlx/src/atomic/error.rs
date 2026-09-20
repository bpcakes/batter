use crate::SqlxFailure;
use std::{error::Error, fmt};

/// Terminal transaction failure. No variant contains a reusable transaction.
pub enum PgTransactionError {
    /// The top-level transaction ended or changed identity.
    TransactionBoundaryLost,
    /// The required transaction isolation changed.
    TransactionIsolationChanged,
    /// The transaction read-only setting changed.
    TransactionAccessChanged,
    /// Original native failure, available for explicit inspection.
    Query(SqlxFailure),
}

impl From<sqlx::Error> for PgTransactionError {
    fn from(value: sqlx::Error) -> Self {
        Self::Query(value.into())
    }
}

impl fmt::Debug for PgTransactionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl fmt::Display for PgTransactionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::TransactionBoundaryLost => "PostgreSQL transaction boundary lost",
            Self::TransactionIsolationChanged => "PostgreSQL transaction isolation changed",
            Self::TransactionAccessChanged => "PostgreSQL transaction access mode changed",
            Self::Query(_) => "PostgreSQL transaction query failed",
        })
    }
}

impl Error for PgTransactionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Query(error) => Some(error),
            _ => None,
        }
    }
}

/// Terminal scope failure, retaining both the application and cleanup causes.
/// Formatting never prints application or database error contents.
pub enum PgScopeError<E> {
    /// Application returned an error; the application scope consumed its owner.
    Application(E),
    /// Setup, continuity, or completion failed after an otherwise successful body.
    Transaction(PgTransactionError),
    /// Application failed and savepoint cleanup or revalidation also failed.
    Recovery {
        /// Original application error.
        application: E,
        /// Original recovery failure.
        recovery: PgTransactionError,
    },
}

impl<E> fmt::Debug for PgScopeError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl<E> fmt::Display for PgScopeError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Application(_) => "PostgreSQL application scope failed",
            Self::Transaction(_) => "PostgreSQL scope transaction failed",
            Self::Recovery { .. } => "PostgreSQL scope and recovery failed",
        })
    }
}

impl<E: Error + 'static> Error for PgScopeError<E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(match self {
            Self::Application(error) => error,
            Self::Transaction(error) => error,
            Self::Recovery { recovery, .. } => recovery,
        })
    }
}

/// Commit was not acknowledged. This never authorizes replay or returns an owner.
/// Cancellation of the commit future produces no returned value at all and must
/// also be reconciled by the application as an uncertain outcome.
pub struct CommitUnconfirmed(pub(crate) PgTransactionError);

impl CommitUnconfirmed {
    /// Inspect the retained cause explicitly.
    pub fn cause(&self) -> &PgTransactionError {
        &self.0
    }

    /// Retain the typed cause when translating into a domain uncertainty error.
    pub fn into_cause(self) -> PgTransactionError {
        self.0
    }
}

impl fmt::Debug for CommitUnconfirmed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl fmt::Display for CommitUnconfirmed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PostgreSQL commit unconfirmed")
    }
}

impl Error for CommitUnconfirmed {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.0)
    }
}
