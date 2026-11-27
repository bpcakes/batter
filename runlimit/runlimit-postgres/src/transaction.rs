//! The one deadline-bounded connection and transaction behind every limiter
//! operation.
//!
//! Quota checks and expired-row cleanup differ only in how they name their
//! phases and classify their failures. A [`Family`] carries that difference,
//! so acquisition, timeout refresh, statement execution, commit and connection
//! disposition exist once.

use std::{future::Future, marker::PhantomData};

use sqlx::{
    Acquire, PgPool, Postgres, Transaction,
    pool::PoolConnection,
    postgres::{PgArguments, PgQueryResult, PgRow},
    query::Query,
};
use tokio::time::{Instant, timeout, timeout_at};

use crate::{
    CheckError, CheckPhase, CleanupPhase, MaintenanceError, PostgresConfig,
    protocol::{is_server_timeout, remaining_server_timeout_settings},
    statements::session::SET_LOCAL_TIMEOUTS_SQL,
};

/// Whether the connection that produced a value may return to its pool.
#[derive(Debug)]
pub(crate) enum ConnectionOutcome<T> {
    Reusable(T),
    MustClose(T),
}

impl<T> ConnectionOutcome<T> {
    pub(crate) fn map<U>(self, map: impl FnOnce(T) -> U) -> ConnectionOutcome<U> {
        match self {
            Self::Reusable(value) => ConnectionOutcome::Reusable(map(value)),
            Self::MustClose(value) => ConnectionOutcome::MustClose(map(value)),
        }
    }
}

/// How one family of bounded operations names its phases and classifies its
/// failures.
pub(crate) trait Family {
    type Phase: Copy;
    type Error;

    const ACQUIRING_CONNECTION: Self::Phase;
    const BEGINNING_TRANSACTION: Self::Phase;
    const PREPARING_COMMIT: Self::Phase;
    const STARTING_COMMIT: Self::Phase;

    /// The phase reported when refreshing the server timeouts ahead of a
    /// statement in `statement` fails.
    fn refreshing_timeouts(statement: Self::Phase) -> Self::Phase;
    fn timed_out_before_commit(phase: Self::Phase) -> Self::Error;
    fn failed_before_commit(error: sqlx::Error) -> Self::Error;
    fn commit_timed_out() -> Self::Error;
    fn commit_outcome_unknown(error: sqlx::Error) -> Self::Error;
}

/// Quota and attempt admission: failures say whether quota was consumed.
pub(crate) enum Check {}

impl Family for Check {
    type Phase = CheckPhase;
    type Error = CheckError;

    const ACQUIRING_CONNECTION: CheckPhase = CheckPhase::AcquiringConnection;
    const BEGINNING_TRANSACTION: CheckPhase = CheckPhase::BeginningTransaction;
    const PREPARING_COMMIT: CheckPhase = CheckPhase::PreparingCommit;
    const STARTING_COMMIT: CheckPhase = CheckPhase::StartingCommit;

    // A check has no separate configuration phase: a refresh that fails is
    // reported as the statement it was preparing.
    fn refreshing_timeouts(statement: CheckPhase) -> CheckPhase {
        statement
    }

    fn timed_out_before_commit(phase: CheckPhase) -> CheckError {
        CheckError::TimedOutBeforeCommit { phase }
    }

    fn failed_before_commit(error: sqlx::Error) -> CheckError {
        CheckError::DefinitelyNotConsumed(error)
    }

    fn commit_timed_out() -> CheckError {
        CheckError::CommitTimedOut
    }

    fn commit_outcome_unknown(error: sqlx::Error) -> CheckError {
        CheckError::CommitOutcomeUnknown(error)
    }
}

/// Expired-row cleanup: failures say whether rows may have been removed.
pub(crate) enum Maintenance {}

impl Family for Maintenance {
    type Phase = CleanupPhase;
    type Error = MaintenanceError;

    const ACQUIRING_CONNECTION: CleanupPhase = CleanupPhase::AcquiringConnection;
    const BEGINNING_TRANSACTION: CleanupPhase = CleanupPhase::BeginningTransaction;
    const PREPARING_COMMIT: CleanupPhase = CleanupPhase::PreparingCommit;
    const STARTING_COMMIT: CleanupPhase = CleanupPhase::StartingCommit;

    fn refreshing_timeouts(_statement: CleanupPhase) -> CleanupPhase {
        CleanupPhase::ConfiguringTimeouts
    }

    fn timed_out_before_commit(phase: CleanupPhase) -> MaintenanceError {
        MaintenanceError::TimedOutBeforeCommit { phase }
    }

    fn failed_before_commit(error: sqlx::Error) -> MaintenanceError {
        MaintenanceError::Database(error)
    }

    fn commit_timed_out() -> MaintenanceError {
        MaintenanceError::CommitTimedOut
    }

    fn commit_outcome_unknown(error: sqlx::Error) -> MaintenanceError {
        MaintenanceError::CommitOutcomeUnknown(error)
    }
}

/// The transaction of a quota or attempt admission.
pub(crate) type CheckTransaction<'c> = BoundedTransaction<'c, Check>;

/// What dropping a guarded connection does with it.
enum OnDrop {
    /// Close the socket: the session may be busy, locked or mid-transaction.
    Close,
    /// Return the idle session to its pool.
    ReturnToPool,
}

/// Retires a pooled connection unless its operation says it is reusable.
pub(crate) struct ConnectionCancellationGuard {
    connection: PoolConnection<Postgres>,
    on_drop: OnDrop,
}

impl ConnectionCancellationGuard {
    pub(crate) const fn new(connection: PoolConnection<Postgres>) -> Self {
        Self {
            connection,
            on_drop: OnDrop::Close,
        }
    }

    pub(crate) const fn connection(&mut self) -> &mut PoolConnection<Postgres> {
        &mut self.connection
    }

    /// Ends the guarded operation, returning the connection to its pool only
    /// when `outcome` says it is reusable.
    pub(crate) fn release<T>(mut self, outcome: ConnectionOutcome<T>) -> T {
        match outcome {
            ConnectionOutcome::Reusable(value) => {
                self.on_drop = OnDrop::ReturnToPool;
                value
            }
            ConnectionOutcome::MustClose(value) => value,
        }
    }
}

impl Drop for ConnectionCancellationGuard {
    fn drop(&mut self) {
        match self.on_drop {
            // Dropping an SQLx query future does not send a PostgreSQL cancel
            // request. Close the socket instead of returning a potentially
            // busy connection to the pool when either Runlimit's deadline or
            // cancellation of the outer limiter future wins the race.
            OnDrop::Close => self.connection.close_on_drop(),
            OnDrop::ReturnToPool => {}
        }
    }
}

/// A guarded pooled connection and the deadline of the one operation it
/// serves.
pub(crate) struct BoundedConnection<F> {
    guard: ConnectionCancellationGuard,
    deadline: Instant,
    family: PhantomData<fn() -> F>,
}

impl<F: Family> BoundedConnection<F> {
    /// Waits for a pooled connection within the acquisition budget.
    ///
    /// Pool contention does not consume the database-work budget. A survivor
    /// receives a fresh deadline for begin, statements, locks, rollback, and
    /// commit.
    pub(crate) async fn acquire(pool: &PgPool, config: PostgresConfig) -> Result<Self, F::Error> {
        let connection = timeout(config.pool_acquire_timeout(), pool.acquire())
            .await
            .map_err(|_| F::timed_out_before_commit(F::ACQUIRING_CONNECTION))?
            .map_err(F::failed_before_commit)?;
        Ok(Self {
            guard: ConnectionCancellationGuard::new(connection),
            deadline: Instant::now() + config.operation_timeout(),
            family: PhantomData,
        })
    }

    pub(crate) async fn begin(
        &mut self,
    ) -> Result<BoundedTransaction<'_, F>, ConnectionOutcome<F::Error>> {
        let deadline = self.deadline;
        let inner = before_commit::<F, _>(
            deadline,
            F::BEGINNING_TRANSACTION,
            self.guard.connection().begin(),
        )
        .await?;
        Ok(BoundedTransaction {
            inner,
            deadline,
            family: PhantomData,
        })
    }

    /// Ends the operation and disposes of the connection as its result says.
    pub(crate) fn finish<T>(
        self,
        result: Result<ConnectionOutcome<T>, ConnectionOutcome<F::Error>>,
    ) -> Result<T, F::Error> {
        match result {
            Ok(outcome) => Ok(self.guard.release(outcome)),
            Err(outcome) => Err(self.guard.release(outcome)),
        }
    }
}

/// Keeps every SQL phase on the same remaining operation budget. `PostgreSQL`
/// timeouts are per statement/lock, so a value set once at BEGIN is stale after
/// an earlier phase waits. Every statement and the commit therefore refresh
/// them first, and no method exposes the underlying transaction.
///
/// The budget is confirmed when each refresh is computed and once more before
/// a commit is attempted. It is not confirmed again between a refresh and the
/// statement that follows it: a statement whose refresh used up the budget is
/// still attempted, and it either completes or times out (`batter-pp9j`).
pub(crate) struct BoundedTransaction<'c, F> {
    inner: Transaction<'c, Postgres>,
    deadline: Instant,
    family: PhantomData<fn() -> F>,
}

impl<F: Family> BoundedTransaction<'_, F> {
    async fn refresh_timeouts(
        &mut self,
        phase: F::Phase,
    ) -> Result<(), ConnectionOutcome<F::Error>> {
        let (statement_timeout, lock_timeout) = remaining_server_timeout_settings(self.deadline)
            .ok_or_else(|| ConnectionOutcome::Reusable(F::timed_out_before_commit(phase)))?;
        before_commit::<F, _>(
            self.deadline,
            phase,
            SET_LOCAL_TIMEOUTS_SQL
                .query()
                .bind(statement_timeout)
                .bind(lock_timeout)
                .execute(&mut *self.inner),
        )
        .await
        .map(|_| ())
    }

    pub(crate) async fn execute(
        &mut self,
        phase: F::Phase,
        query: Query<'_, Postgres, PgArguments>,
    ) -> Result<PgQueryResult, ConnectionOutcome<F::Error>> {
        self.refresh_timeouts(F::refreshing_timeouts(phase)).await?;
        before_commit::<F, _>(self.deadline, phase, query.execute(&mut *self.inner)).await
    }

    pub(crate) async fn fetch_all(
        &mut self,
        phase: F::Phase,
        query: Query<'_, Postgres, PgArguments>,
    ) -> Result<Vec<PgRow>, ConnectionOutcome<F::Error>> {
        self.refresh_timeouts(F::refreshing_timeouts(phase)).await?;
        before_commit::<F, _>(self.deadline, phase, query.fetch_all(&mut *self.inner)).await
    }

    pub(crate) async fn fetch_one(
        &mut self,
        phase: F::Phase,
        query: Query<'_, Postgres, PgArguments>,
    ) -> Result<PgRow, ConnectionOutcome<F::Error>> {
        self.refresh_timeouts(F::refreshing_timeouts(phase)).await?;
        before_commit::<F, _>(self.deadline, phase, query.fetch_one(&mut *self.inner)).await
    }

    /// Commits within the remaining budget: refreshes the server timeouts,
    /// then hands the commit to [`attempt_commit`], which confirms that budget
    /// remains before it polls the commit at all.
    pub(crate) async fn commit(mut self) -> Result<(), ConnectionOutcome<F::Error>> {
        self.refresh_timeouts(F::PREPARING_COMMIT).await?;
        // `Transaction::commit` does nothing until polled, and dropping it
        // unpolled rolls the transaction back like dropping the transaction.
        attempt_commit::<F>(self.deadline, self.inner.commit()).await
    }
}

/// Attempts `commit` only when budget remains at this instant.
///
/// With the deadline already passed, `commit` is dropped without being polled
/// and the failure is a definite pre-commit timeout. From the first poll on, a
/// deadline or an error is reported as an uncertain outcome, whether or not
/// `COMMIT` reached the server: the client cannot tell the two apart.
async fn attempt_commit<F>(
    deadline: Instant,
    commit: impl Future<Output = Result<(), sqlx::Error>>,
) -> Result<(), ConnectionOutcome<F::Error>>
where
    F: Family,
{
    if Instant::now() >= deadline {
        return Err(ConnectionOutcome::Reusable(F::timed_out_before_commit(
            F::STARTING_COMMIT,
        )));
    }
    timeout_at(deadline, commit)
        .await
        .map_err(|_| ConnectionOutcome::MustClose(F::commit_timed_out()))?
        .map_err(|error| ConnectionOutcome::Reusable(F::commit_outcome_unknown(error)))
}

impl BoundedTransaction<'_, Check> {
    /// Rolls back a transaction whose denial is already a valid decision.
    pub(crate) async fn deny<D>(self, decision: D) -> ConnectionOutcome<D> {
        finish_denied_transaction(self.deadline, decision, self.inner.rollback()).await
    }
}

/// Rolls back a denied transaction and reports whether the connection can be
/// reused.
///
/// The denial is already a valid decision, so a rollback failure or deadline
/// never replaces it; the connection is discarded instead so closing it rolls
/// the non-mutating transaction back.
pub(crate) async fn finish_denied_transaction<D, R>(
    deadline: Instant,
    decision: D,
    rollback: R,
) -> ConnectionOutcome<D>
where
    R: Future<Output = Result<(), sqlx::Error>>,
{
    match timeout_at(deadline, rollback).await {
        Ok(Ok(())) => ConnectionOutcome::Reusable(decision),
        Ok(Err(_)) | Err(_) => ConnectionOutcome::MustClose(decision),
    }
}

/// Bounds one pre-commit step. A client deadline leaves the session busy, so
/// the connection must close; a server-side timeout or error leaves it idle.
async fn before_commit<F, T>(
    deadline: Instant,
    phase: F::Phase,
    step: impl Future<Output = Result<T, sqlx::Error>>,
) -> Result<T, ConnectionOutcome<F::Error>>
where
    F: Family,
{
    timeout_at(deadline, step)
        .await
        .map_err(|_| ConnectionOutcome::MustClose(F::timed_out_before_commit(phase)))?
        .map_err(|error| {
            ConnectionOutcome::Reusable(if is_server_timeout(&error) {
                F::timed_out_before_commit(phase)
            } else {
                F::failed_before_commit(error)
            })
        })
}

#[cfg(test)]
mod tests;
