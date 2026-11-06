use crate::{
    Archive, Change, CommitUncertainty, ErrorClass, PooledStore, StoreError, Write, WriteOutcome,
    errors::ErrorKind,
    store::{bounded, remaining},
};
use batter_flags_core::FlagKey;
use batter_sqlx::{PgScopedSql, PgTransactionError, low_level::PgAtomicTransaction};
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
impl PooledStore {
    /// Write exactly once under the global counter lock. Every refusal and no-op
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
        let acquire_deadline = deadline.min(Instant::now() + self.timing.acquire);
        let tx = bounded(acquire_deadline, async {
            PgAtomicTransaction::begin(&self.pool)
                .await
                .map_err(StoreError::from)
        })
        .await?;
        let (tx, result) = bounded(deadline, async {
            tx.operation(async |sql| self.prepare(sql, &mutation, deadline).await)
                .await
                .map_err(|e| StoreError(ErrorKind::Scope(Box::new(e))))
        })
        .await?;
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
                    Err(cleanup) => Err(StoreError(ErrorKind::Other(Box::new(RollbackFailure {
                        error,
                        cleanup,
                    })))),
                };
            }
        };
        let proposed_version = match &outcome {
            WriteOutcome::Written { version, .. }
            | WriteOutcome::WrittenUndeclared { version, .. } => *version,
            _ => {
                let _ = bounded(deadline, async {
                    tx.rollback().await.map_err(StoreError::from)
                })
                .await?;
                return Ok(outcome);
            }
        };
        // Do not poll commit at all after a known pre-commit timeout.
        remaining(deadline)?;
        match tokio::time::timeout_at(deadline, tx.commit()).await {
            Ok(Ok(_)) => Ok(outcome),
            Ok(Err(cause)) if known_commit_rejection(cause.cause()) => {
                Err(cause.into_cause().into())
            }
            Ok(Err(cause)) => Ok(WriteOutcome::Uncertain {
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
        deadline: Instant,
    ) -> Result<WriteOutcome, StoreError> {
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
        super::validate::apply(sql, mutation, head, next).await
    }
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
