use crate::{
    PgLease, PgScopedSql, PgTransactionError,
    atomic::scope::{ScopeSavepoint, command},
};
use sqlx::PgPool;
use std::{error::Error, fmt};

/// Owned read-only inspection on one REPEATABLE READ snapshot.
///
/// Acquires a fresh lease, normalizes it to idle, begins the transaction and
/// returns the inspection result only after rollback is acknowledged. All query
/// names and domain compatibility decisions remain the inspecting library's
/// responsibility. PostgreSQL catalog cache functions and external state do not
/// necessarily obey an MVCC snapshot: use direct qualified catalog reads.
///
/// ```no_run
/// # async fn example(pool: &sqlx::PgPool) -> Result<(), Box<dyn std::error::Error>> {
/// let count = batter_sqlx::PgReadOnlySnapshot::inspect(pool, async |sql| {
///     sqlx::query_scalar::<_, i64>("SELECT count(*) FROM pg_catalog.pg_class")
///         .fetch_one(sql.executor()).await
/// }).await?;
/// # let _ = count;
/// # Ok(()) }
/// ```
pub struct PgReadOnlySnapshot;

/// Inspection and cleanup failures, with original causes explicitly retained.
pub enum PgSnapshotError<E> {
    /// Inspection returned an error after successful rollback.
    Inspection(E),
    /// Transaction setup, continuity or rollback failed.
    Transaction(PgTransactionError),
    /// Both inspection and rollback failed.
    Cleanup {
        /// Original inspection error.
        inspection: E,
        /// Original rollback failure.
        cleanup: PgTransactionError,
    },
}

impl PgReadOnlySnapshot {
    /// Run one inspection; cancellation or panic retires the owned connection.
    /// Use this for observation, never for application writes. No caller-owned
    /// transaction is accepted or rolled back by this API.
    pub async fn inspect<T, E>(
        pool: &PgPool,
        inspect: impl AsyncFnOnce(&mut PgScopedSql<'_>) -> Result<T, E>,
    ) -> Result<T, PgSnapshotError<E>> {
        let mut lease = PgLease {
            connection: Some(
                pool.acquire()
                    .await
                    .map_err(PgTransactionError::from)
                    .map_err(PgSnapshotError::Transaction)?,
            ),
        };
        let setup = async {
            command(lease.connection_mut(), "ROLLBACK").await?;
            command(
                lease.connection_mut(),
                "BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY",
            )
            .await?;
            command(
                lease.connection_mut(),
                "SET LOCAL search_path = pg_catalog, pg_temp",
            )
            .await?;
            // No SELECT before the inspector: it may lock authoritative objects
            // before its first query establishes the snapshot.
            ScopeSavepoint::begin(lease.connection_mut()).await
        }
        .await;
        let guard = setup.map_err(PgSnapshotError::Transaction)?;
        let result = inspect(&mut PgScopedSql {
            connection: lease.connection_mut(),
        })
        .await;
        match result {
            Ok(value) => {
                let (isolation, read_only): (String, String) = sqlx::query_as(
                    "SELECT pg_catalog.current_setting('transaction_isolation'), \
                     pg_catalog.current_setting('transaction_read_only')",
                )
                .fetch_one(lease.connection_mut())
                .await
                .map_err(PgTransactionError::from)
                .map_err(PgSnapshotError::Transaction)?;
                if isolation != "repeatable read" || read_only != "on" {
                    return Err(PgSnapshotError::Transaction(
                        PgTransactionError::TransactionBoundaryLost,
                    ));
                }
                guard
                    .release(lease.connection_mut())
                    .await
                    .map_err(PgSnapshotError::Transaction)?;
                command(lease.connection_mut(), "ROLLBACK")
                    .await
                    .map_err(PgSnapshotError::Transaction)?;
                drop(lease.connection.take());
                Ok(value)
            }
            Err(inspection) => {
                // Even a successful rollback cannot undo arbitrary session state
                // or an early raw COMMIT; failure paths always retire the lease.
                match command(lease.connection_mut(), "ROLLBACK").await {
                    Ok(()) => Err(PgSnapshotError::Inspection(inspection)),
                    Err(cleanup) => Err(PgSnapshotError::Cleanup {
                        inspection,
                        cleanup,
                    }),
                }
            }
        }
    }
}

impl<E> fmt::Debug for PgSnapshotError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl<E> fmt::Display for PgSnapshotError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PostgreSQL snapshot inspection failed")
    }
}

impl<E: Error + 'static> Error for PgSnapshotError<E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(match self {
            Self::Inspection(error) => error,
            Self::Transaction(error) => error,
            Self::Cleanup { cleanup, .. } => cleanup,
        })
    }
}
