use crate::SqlxFailure;
use std::{error::Error, fmt, sync::Arc};

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

/// An operation rejection or terminal scope failure. Only `Application` permits
/// the canonical runner to keep using the recovered transaction.
pub enum PgScopeError<E> {
    /// Application returned an error. This alone is not outer rollback evidence;
    /// the canonical runner supplies the final disposition.
    Application(E),
    /// The transaction is no longer usable; the runner retains its loss cause.
    Terminal(PgScopeFailure<E>),
}

/// Why a scope cannot acknowledge disposition. Shared native causes keep their
/// identity even when the callback ignores, translates, or retries an error.
#[derive(Clone)]
pub enum PgScopeLoss {
    /// A polled operation was dropped before it returned a usable owner.
    OperationAbandoned,
    /// Original terminal database failure, not a synthesized replacement.
    Transaction(Arc<PgTransactionError>),
}

/// Only terminal scope failures; an ordinary application rejection cannot be
/// wrapped in this type. Recovery retains the concrete application error too.
/// ```compile_fail,E0308
/// use batter_sqlx::{PgScopeError, PgScopeFailure};
/// let terminal: PgScopeFailure<()> = PgScopeError::Application(());
/// ```
pub enum PgScopeFailure<E> {
    /// Setup, continuity, or completion failed after an otherwise successful body.
    Transaction(Arc<PgTransactionError>),
    /// Application failed and savepoint cleanup or revalidation also failed.
    Recovery {
        /// Original application error.
        application: E,
        /// Original recovery failure.
        recovery: Arc<PgTransactionError>,
    },
    /// A previous polled operation was abandoned, without a returned DB error.
    OperationAbandoned,
}

impl<E> PgScopeFailure<E> {
    pub(crate) fn loss(&self) -> PgScopeLoss {
        match self {
            Self::Transaction(cause)
            | Self::Recovery {
                recovery: cause, ..
            } => PgScopeLoss::Transaction(Arc::clone(cause)),
            Self::OperationAbandoned => PgScopeLoss::OperationAbandoned,
        }
    }
}

impl<E> From<PgTransactionError> for PgScopeFailure<E> {
    fn from(cause: PgTransactionError) -> Self {
        Self::Transaction(Arc::new(cause))
    }
}

impl<E> From<PgScopeLoss> for PgScopeFailure<E> {
    fn from(cause: PgScopeLoss) -> Self {
        match cause {
            PgScopeLoss::Transaction(cause) => Self::Transaction(cause),
            PgScopeLoss::OperationAbandoned => Self::OperationAbandoned,
        }
    }
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
            Self::Terminal(_) => "PostgreSQL scope terminated",
        })
    }
}

impl<E: Error + 'static> Error for PgScopeError<E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(match self {
            Self::Application(error) => error,
            Self::Terminal(error) => error,
        })
    }
}

impl fmt::Debug for PgScopeLoss {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl fmt::Display for PgScopeLoss {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::OperationAbandoned => "PostgreSQL scope operation abandoned",
            Self::Transaction(_) => "PostgreSQL scope transaction lost",
        })
    }
}

impl Error for PgScopeLoss {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::OperationAbandoned => None,
            Self::Transaction(cause) => Some(cause.as_ref()),
        }
    }
}

impl<E> fmt::Debug for PgScopeFailure<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl<E> fmt::Display for PgScopeFailure<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Transaction(_) => "PostgreSQL scope transaction failed",
            Self::Recovery { .. } => "PostgreSQL scope and recovery failed",
            Self::OperationAbandoned => "PostgreSQL scope operation abandoned",
        })
    }
}

impl<E: Error + 'static> Error for PgScopeFailure<E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Transaction(cause)
            | Self::Recovery {
                recovery: cause, ..
            } => Some(cause.as_ref()),
            Self::OperationAbandoned => None,
        }
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
