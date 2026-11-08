use crate::{
    Archive, Change, CommitUncertainty, ErrorClass, PooledStore, StoreError, Write, WriteOutcome,
    errors::ErrorKind,
    prepared::Prepared,
    store::{bounded, remaining},
};
use batter_flags_core::FlagKey;
use batter_sqlx::{PgCommitStop, PgScopedSql, PgTransactionError, low_level::PgAtomicTransaction};
use sqlx::Row;
use tokio::time::Instant;

pub(crate) enum Mutation {
    Write(Write),
    Archive(Archive),
    Restore(Archive),
}
impl Mutation {
    pub(crate) fn key(&self) -> &FlagKey {
        match self {
            Self::Write(w) => &w.key,
            Self::Archive(a) | Self::Restore(a) => &a.key,
        }
    }
    pub(crate) fn change(&self) -> &Change {
        match self {
            Self::Write(w) => &w.change,
            Self::Archive(a) | Self::Restore(a) => &a.change,
        }
    }
}
/// Whether a write may flip a key a retained catalog declares coordinated.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FlipPolicy {
    /// Refuse it with [`Refusal::CoordinatedFlip`](crate::Refusal::CoordinatedFlip),
    /// judged inside the write's transaction.
    RefuseCoordinated,
    /// Write it; processes apply it poll by poll.
    AcceptRolling,
}

/// One audited mutation, for [`PooledStore::submit`].
#[derive(Clone, Debug)]
pub enum Request {
    /// `set`, `kill` or an explicit write.
    Write(Write),
    /// Archive an existing key.
    Archive(Archive),
    /// Restore an archived key.
    Restore(Archive),
}

impl PooledStore {
    /// Runs one audited mutation that `interrupt` may stop before COMMIT is
    /// sent: a signal while the transaction is being prepared, or before
    /// COMMIT, rolls it back and returns [`ErrorClass::Interrupted`], with
    /// nothing written. Once COMMIT is sent it is never interrupted, so the
    /// outcome reported is the real one. Dropping the sender interrupts
    /// nothing.
    pub async fn submit(
        &self,
        request: Request,
        flip: FlipPolicy,
        deadline: Instant,
        interrupt: tokio::sync::oneshot::Receiver<()>,
    ) -> Result<WriteOutcome, StoreError> {
        let mutation = match request {
            Request::Write(write) => Mutation::Write(write),
            Request::Archive(archive) => Mutation::Archive(archive),
            Request::Restore(restore) => Mutation::Restore(restore),
        };
        self.mutate_until(mutation, flip, deadline, Interrupt::On(interrupt))
            .await
    }

    /// Write exactly once under the global counter lock. It never refuses a
    /// coordinated flip; [`submit`](Self::submit) with
    /// [`FlipPolicy::RefuseCoordinated`] does. Every refusal and no-op
    /// acknowledges rollback. No transport or timeout result triggers retry.
    pub async fn write(&self, write: Write, deadline: Instant) -> Result<WriteOutcome, StoreError> {
        self.mutate(Mutation::Write(write), deadline).await
    }
    /// Archive an existing key only after all Live/Silent declarers disappear.
    /// Already archived or absent keys are unchanged; ordinary writes never restore.
    pub async fn archive(
        &self,
        archive: Archive,
        deadline: Instant,
    ) -> Result<WriteOutcome, StoreError> {
        self.mutate(Mutation::Archive(archive), deadline).await
    }
    /// Clear an existing key's archive marker after fleet/CAS validation.
    pub async fn restore(
        &self,
        restore: Archive,
        deadline: Instant,
    ) -> Result<WriteOutcome, StoreError> {
        self.mutate(Mutation::Restore(restore), deadline).await
    }
    async fn mutate(
        &self,
        mutation: Mutation,
        deadline: Instant,
    ) -> Result<WriteOutcome, StoreError> {
        self.mutate_until(
            mutation,
            FlipPolicy::AcceptRolling,
            deadline,
            Interrupt::Never,
        )
        .await
    }

    async fn mutate_until(
        &self,
        mutation: Mutation,
        flip: FlipPolicy,
        deadline: Instant,
        mut interrupt: Interrupt,
    ) -> Result<WriteOutcome, StoreError> {
        let acquire_deadline = deadline.min(Instant::now() + self.timing.acquire);
        let tx = bounded(acquire_deadline, async {
            PgAtomicTransaction::begin(&self.pool)
                .await
                .map_err(StoreError::from)
        })
        .await?;
        let operation = tx.operation_until(deadline, async |sql| {
            self.prepare(sql, &mutation, flip, deadline).await
        });
        // Until COMMIT is sent an interrupt drops the transaction whole: its
        // connection is retired, so the server aborts it, and nothing waits
        // for a statement still blocked on a lock.
        let (tx, result) = match &mut interrupt {
            Interrupt::Never => operation.await,
            Interrupt::On(signal) => tokio::select! {
                biased;
                Ok(()) = signal => return Err(StoreError::classified(ErrorClass::Interrupted)),
                done = operation => done,
            },
        }
        .map_err(crate::atomic::scope_error)?;
        let outcome = match result {
            Ok(outcome) => outcome,
            Err(error) => {
                // Preserve both causes if rollback fails. The private body cannot commit.
                return match bounded(deadline, async {
                    tx.rollback().await.map_err(StoreError::from)
                })
                .await
                {
                    Ok(_) => Err(error),
                    Err(cleanup) => Err(RollbackFailure::both(error, cleanup)),
                };
            }
        };
        let pending = match outcome {
            Prepared::Commit(pending) => pending,
            Prepared::Rollback(decision) => {
                let proof = bounded(deadline, async {
                    tx.rollback().await.map_err(StoreError::from)
                })
                .await?;
                return Ok(decision.completed(proof));
            }
        };
        // Do not poll commit at all after a known pre-commit timeout.
        remaining(deadline)?;
        let proposed_version = pending.version;
        // An interrupt is honoured until COMMIT is sent, including during the
        // native continuity check before it, and never after.
        let committed = match interrupt {
            Interrupt::Never => tokio::time::timeout_at(deadline, tx.commit())
                .await
                .map(|result| result.map_err(PgCommitStop::Unconfirmed)),
            Interrupt::On(signal) => {
                let stop = async move {
                    if signal.await.is_err() {
                        std::future::pending::<()>().await;
                    }
                };
                tokio::time::timeout_at(deadline, tx.commit_unless(stop)).await
            }
        };
        match committed {
            Ok(Ok(proof)) => Ok(pending.committed(proof)),
            Ok(Err(PgCommitStop::Interrupted)) => {
                Err(StoreError::classified(ErrorClass::Interrupted))
            }
            Ok(Err(PgCommitStop::Unconfirmed(cause))) if known_commit_rejection(cause.cause()) => {
                Err(cause.into_cause().into())
            }
            Ok(Err(PgCommitStop::Unconfirmed(cause))) => Ok(WriteOutcome::Uncertain {
                proposed_version,
                cause: CommitUncertainty::Native(cause),
            }),
            Err(_) => Ok(WriteOutcome::Uncertain {
                proposed_version,
                cause: CommitUncertainty::Deadline,
            }),
        }
    }
    async fn prepare(
        &self,
        sql: &mut PgScopedSql<'_>,
        mutation: &Mutation,
        flip: FlipPolicy,
        deadline: Instant,
    ) -> Result<Prepared, StoreError> {
        let [statement, lock, idle] = self.timing.settings(deadline)?;
        sqlx::query("SELECT set_config('statement_timeout',$1,true), set_config('lock_timeout',$2,true), set_config('idle_in_transaction_session_timeout',$3,true), set_config('batter_flags.actor',$4,true), set_config('batter_flags.reason',$5,true), set_config('batter_flags.guard',$6,true)")
            .bind(statement).bind(lock).bind(idle).bind(mutation.change().actor.as_str())
            .bind(mutation.change().reason.as_str()).bind(mutation.change().precondition.label())
            .execute(sql.executor()).await?;
        let row = sqlx::query(
            "SELECT version, schema_revision FROM flags.flag_control WHERE singleton FOR UPDATE",
        )
        .fetch_one(sql.executor())
        .await?;
        if row.try_get::<i32, _>("schema_revision")? < crate::REQUIRED_SCHEMA_REVISION {
            return Err(StoreError::classified(ErrorClass::SchemaRevision));
        }
        let head: i64 = row.try_get("version")?;
        crate::load::version(head)?;
        let next = head
            .checked_add(1)
            .ok_or_else(|| StoreError::classified(ErrorClass::InvalidData))?;
        sqlx::query("UPDATE flags.flag_control SET version = $1 WHERE singleton")
            .bind(next)
            .execute(sql.executor())
            .await?;
        #[cfg(test)]
        if let Some(pause) = &self.pause {
            pause.entered.notify_one();
            pause.release.notified().await;
        }
        super::validate::apply(sql, mutation, flip, head, next).await
    }
}
/// Whether a mutation may be interrupted before COMMIT.
enum Interrupt {
    Never,
    On(tokio::sync::oneshot::Receiver<()>),
}

// SQLSTATE ErrorResponse on COMMIT is a rejection, except connection/unknown
// completion classes: these cannot prove the transaction's outcome.
pub(crate) fn known_commit_rejection(error: &PgTransactionError) -> bool {
    match error {
        PgTransactionError::Query(failure) => failure
            .native()
            .as_database_error()
            .and_then(|e| e.code())
            .is_some_and(|code| {
                !code.starts_with("08")
                    && !matches!(code.as_ref(), "40003" | "57P01" | "57P02" | "57P03")
            }),
        _ => false,
    }
}
/// Both failures retained when an operation and its rollback fail.
pub struct RollbackFailure {
    error: StoreError,
    cleanup: StoreError,
}
impl RollbackFailure {
    pub(crate) fn both(error: StoreError, cleanup: StoreError) -> StoreError {
        StoreError(ErrorKind::Other(Box::new(Self { error, cleanup })))
    }
    pub(crate) fn deadline(error: StoreError) -> StoreError {
        StoreError(ErrorKind::Other(Box::new(Self {
            error,
            cleanup: StoreError::classified(ErrorClass::Deadline),
        })))
    }
    /// Original operation failure.
    pub fn operation(&self) -> &StoreError {
        &self.error
    }
    /// Rollback failure.
    pub fn cleanup(&self) -> &StoreError {
        &self.cleanup
    }
}
impl std::fmt::Debug for RollbackFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("flags rollback failed after operation failure")
    }
}
impl std::fmt::Display for RollbackFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(self, f)
    }
}
impl std::error::Error for RollbackFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.error)
    }
}
