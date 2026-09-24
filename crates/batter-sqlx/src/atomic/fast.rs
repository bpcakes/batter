use super::scope::ScopeSavepoint;
use super::{PgAtomicTransaction, PgRollbackConfirmed, PgScopeFailure, PgScopedSql};
use crate::PgTransactionError;

#[allow(
    clippy::large_enum_variant,
    reason = "keep the already-owned transaction inline instead of allocating on every successful fast operation"
)]
pub(crate) enum FastOperation<T, E> {
    Continue(PgAtomicTransaction, T),
    Rejected(PgRollbackConfirmed, E),
}

impl PgAtomicTransaction {
    // One guard at birth, not one per operation. A failed transaction cannot
    // answer the XID query; rolling back this guard restores that capability
    // without accepting a replacement transaction as the original one.
    pub(crate) async fn begin_fast_guard(&mut self) -> Result<ScopeSavepoint, PgTransactionError> {
        ScopeSavepoint::begin(self.lease.connection_mut()).await
    }

    pub(crate) async fn fast_operation<T, E>(
        mut self,
        guard: &ScopeSavepoint,
        work: impl AsyncFnOnce(&mut PgScopedSql<'_>) -> Result<T, E>,
    ) -> Result<FastOperation<T, E>, PgScopeFailure<E>> {
        if self.profile.is_some() {
            self.validate().await?;
        }
        match work(&mut PgScopedSql {
            connection: self.lease.connection_mut(),
        })
        .await
        {
            Ok(value) => {
                self.validate().await?;
                Ok(FastOperation::Continue(self, value))
            }
            Err(application) => {
                let rollback = async {
                    guard.rollback(self.lease.connection_mut()).await?;
                    self.rollback().await
                }
                .await;
                match rollback {
                    Ok(confirmed) => Ok(FastOperation::Rejected(confirmed, application)),
                    Err(recovery) => Err(PgScopeFailure::Recovery {
                        application,
                        recovery: recovery.into(),
                    }),
                }
            }
        }
    }
}
