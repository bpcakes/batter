use crate::{
    PgScopeError, PgScopeLoss, PgScopedSql, PgTransactionError, atomic::PgAtomicTransaction,
};
use sqlx::PgPool;
use std::{error::Error, fmt};

/// Execute one atomic workflow, releasing its output only after COMMIT is
/// acknowledged. A rejection is returned only after acknowledged ROLLBACK.
/// Uncertainty retains the body's result without presenting it as committed.
///
/// Bound this entire future with an OperationContext when needed. Cancellation
/// or panic retires the session and returns no output; neither proves rollback.
/// There is no await after completion acknowledgement. Deliberate external side
/// effects or exporting values through captured state are not sandboxed.
///
/// ```no_run
/// # async fn example(pool: &sqlx::PgPool) -> Result<(), Box<dyn std::error::Error>> {
/// let id = batter_sqlx::run_atomic(pool, async |scope| {
///     scope.application(async |sql| {
///         sqlx::query_scalar::<_, i64>("INSERT INTO audit_events DEFAULT VALUES RETURNING id")
///             .fetch_one(sql.executor()).await
///     }).await
/// }).await?;
/// // Only an acknowledged commit reaches here.
/// # let _ = id;
/// # Ok(()) }
/// ```
/// The scope cannot escape or be completed separately from its result:
/// ```compile_fail
/// # async fn example(pool: &sqlx::PgPool) {
/// let scope = batter_sqlx::run_atomic(pool, async |scope| Ok::<_, ()>(scope)).await;
/// # }
/// ```
/// ```compile_fail,E0599
/// async fn rollback(scope: &mut batter_sqlx::PgAtomicScope) {
///     scope.rollback().await;
/// }
/// ```
pub async fn run_atomic<T, E>(
    pool: &PgPool,
    work: impl AsyncFnOnce(&mut PgAtomicScope) -> Result<T, E>,
) -> Result<T, PgAtomicError<T, E>> {
    let owner = PgAtomicTransaction::begin(pool)
        .await
        .map_err(PgAtomicError::Begin)?;
    run_owned(owner, work).await
}

/// Execute with declared role/path/settings restored and verified after reset,
/// before BEGIN and before invoking the body. Setup failure never runs the body.
pub async fn run_atomic_profiled<T, E>(
    pool: &PgPool,
    profile: &crate::PgSessionProfile,
    work: impl AsyncFnOnce(&mut PgAtomicScope) -> Result<T, E>,
) -> Result<T, PgAtomicError<T, E>> {
    let owner = PgAtomicTransaction::begin_profiled(pool, profile)
        .await
        .map_err(PgAtomicError::Begin)?;
    run_owned(owner, work).await
}

async fn run_owned<T, E>(
    owner: PgAtomicTransaction,
    work: impl AsyncFnOnce(&mut PgAtomicScope) -> Result<T, E>,
) -> Result<T, PgAtomicError<T, E>> {
    let mut scope = PgAtomicScope::new(owner);
    let result = work(&mut scope).await;
    scope.finish(result).await
}

/// SQL operations available only inside [`run_atomic`]. No completion or owner
/// extraction is exposed. Cancelling a polled operation permanently consumes
/// the usable state, even if the callback catches the cancellation and returns.
pub struct PgAtomicScope {
    state: ScopeState,
}

enum ScopeState {
    Live(PgAtomicTransaction),
    InFlight,
    Poisoned(PgScopeLoss),
}

impl PgAtomicScope {
    pub(crate) fn new(owner: PgAtomicTransaction) -> Self {
        Self {
            state: ScopeState::Live(owner),
        }
    }

    pub(crate) async fn finish<T, E>(self, result: Result<T, E>) -> Result<T, PgAtomicError<T, E>> {
        let owner = match self.state {
            ScopeState::Live(owner) => owner,
            ScopeState::InFlight => {
                return Err(PgAtomicError::Uncertain(PgAtomicUncertainty::ScopeLost {
                    result,
                    cause: PgScopeLoss::OperationAbandoned,
                }));
            }
            ScopeState::Poisoned(cause) => {
                return Err(PgAtomicError::Uncertain(PgAtomicUncertainty::ScopeLost {
                    result,
                    cause,
                }));
            }
        };
        match result {
            Ok(output) => match owner.commit().await {
                Ok(_) => Ok(output),
                Err(cause) => Err(PgAtomicError::Uncertain(
                    PgAtomicUncertainty::CommitUnconfirmed {
                        output,
                        cause: cause.into_cause(),
                    },
                )),
            },
            Err(rejection) => match owner.rollback().await {
                Ok(_) => Err(PgAtomicError::Rejected(rejection)),
                Err(cause) => Err(PgAtomicError::Uncertain(
                    PgAtomicUncertainty::RollbackUnconfirmed { rejection, cause },
                )),
            },
        }
    }

    pub(crate) fn take_owner(&mut self) -> Result<PgAtomicTransaction, PgScopeLoss> {
        match std::mem::replace(&mut self.state, ScopeState::InFlight) {
            ScopeState::Live(owner) => Ok(owner),
            state => {
                let cause = match state {
                    ScopeState::InFlight => PgScopeLoss::OperationAbandoned,
                    ScopeState::Poisoned(cause) => cause,
                    ScopeState::Live(_) => unreachable!(),
                };
                self.poison(cause.clone());
                Err(cause)
            }
        }
    }

    pub(crate) fn restore(&mut self, owner: PgAtomicTransaction) {
        self.state = ScopeState::Live(owner);
    }

    pub(crate) fn poison(&mut self, cause: PgScopeLoss) {
        self.state = ScopeState::Poisoned(cause);
    }

    /// Execute inside a private savepoint. A normal application error rolls back
    /// that operation; a cleanup failure retains both causes and poisons the scope.
    /// Values are provisional inside the callback, not durable results yet.
    pub async fn application<T, E>(
        &mut self,
        work: impl AsyncFnOnce(&mut PgScopedSql<'_>) -> Result<T, E>,
    ) -> Result<T, PgScopeError<E>> {
        let owner = self
            .take_owner()
            .map_err(|cause| PgScopeError::Terminal(cause.into()))?;
        match owner.operation(work).await {
            Ok((owner, result)) => {
                self.restore(owner);
                result.map_err(PgScopeError::Application)
            }
            Err(failure) => {
                self.poison(failure.loss());
                Err(PgScopeError::Terminal(failure))
            }
        }
    }
}

/// Exhaustive disposition of a whole atomic workflow. Formatting is redacted;
/// domain outputs and original causes are retained for explicit reconciliation.
pub enum PgAtomicError<T, E> {
    /// Acquisition or setup failed before the callback was invoked.
    Begin(PgTransactionError),
    /// Domain rejection after acknowledged rollback of the original transaction.
    Rejected(E),
    /// Disposition was not acknowledged; retains provisional output/rejection
    /// and its original failure or explicit abandonment reason.
    Uncertain(PgAtomicUncertainty<T, E>),
}

/// Only unacknowledged dispositions. Known begin failures and acknowledged
/// rejections cannot be constructed as members of this type.
/// ```compile_fail,E0308
/// use batter_sqlx::{PgAtomicError, PgAtomicUncertainty};
/// let uncertain: PgAtomicUncertainty<(), ()> = PgAtomicError::Rejected(());
/// ```
pub enum PgAtomicUncertainty<T, E> {
    /// The body succeeded, but commit was not acknowledged. No automatic replay.
    CommitUnconfirmed {
        /// Provisional body output; not evidence of durable effects.
        output: T,
        /// Original completion failure.
        cause: PgTransactionError,
    },
    /// The body rejected, but rollback was not acknowledged.
    RollbackUnconfirmed {
        /// Original domain rejection.
        rejection: E,
        /// Original completion failure.
        cause: PgTransactionError,
    },
    /// An operation lost its owner through cancellation or a terminal failure.
    /// The callback result is retained, but no disposition can be confirmed.
    ScopeLost {
        /// Provisional result, including any returned operation/cleanup causes.
        result: Result<T, E>,
        /// First terminal cause, or explicit abandonment when no error returned.
        cause: PgScopeLoss,
    },
}

impl<T, E> fmt::Debug for PgAtomicError<T, E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
impl<T, E> fmt::Display for PgAtomicError<T, E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Begin(_) => "PostgreSQL atomic workflow setup failed",
            Self::Rejected(_) => "PostgreSQL atomic workflow rejected and rolled back",
            Self::Uncertain(_) => "PostgreSQL atomic workflow disposition unconfirmed",
        })
    }
}
impl<T, E: Error + 'static> Error for PgAtomicError<T, E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Begin(cause) => Some(cause),
            Self::Rejected(error) => Some(error),
            Self::Uncertain(error) => error.source(),
        }
    }
}

impl<T, E> fmt::Debug for PgAtomicUncertainty<T, E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl<T, E> fmt::Display for PgAtomicUncertainty<T, E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::CommitUnconfirmed { .. } => "PostgreSQL atomic workflow commit unconfirmed",
            Self::RollbackUnconfirmed { .. } => "PostgreSQL atomic workflow rollback unconfirmed",
            Self::ScopeLost { .. } => "PostgreSQL atomic workflow scope lost",
        })
    }
}

impl<T, E: Error + 'static> Error for PgAtomicUncertainty<T, E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(match self {
            Self::CommitUnconfirmed { cause, .. } | Self::RollbackUnconfirmed { cause, .. } => {
                cause
            }
            Self::ScopeLost { cause, .. } => cause,
        })
    }
}
