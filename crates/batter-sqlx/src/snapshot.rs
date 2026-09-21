use crate::{
    PgLease, PgTransactionError,
    atomic::scope::{ScopeSavepoint, command, normalize},
};
use sqlx::{Executor, PgConnection, PgPool, Postgres};
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

/// Read-only SQL capability, distinct from write-oriented [`crate::PgScopedSql`]
/// and [`crate::PgExecutor`]. PostgreSQL enforces arbitrary SQL text's read-only
/// restrictions; this type prevents accidental composition with write helpers.
///
/// ```compile_fail,E0451
/// fn forge(connection: &mut sqlx::PgConnection) {
///     let _ = batter_sqlx::PgReadOnlySql { connection };
/// }
/// ```
/// ```compile_fail,E0308
/// async fn writer(sql: &mut batter_sqlx::PgScopedSql<'_>) {}
/// # async fn example(pool: &sqlx::PgPool) {
/// batter_sqlx::PgReadOnlySnapshot::inspect(pool, async |sql| {
///     writer(sql).await;
///     Ok::<_, ()>(())
/// }).await;
/// # }
/// ```
pub struct PgReadOnlySql<'a> {
    connection: &'a mut PgConnection,
}

impl PgReadOnlySql<'_> {
    /// Execute SQL on the retained read-only snapshot. No native resource escapes.
    pub fn executor(&mut self) -> impl Executor<'_, Database = Postgres> {
        &mut *self.connection
    }
}

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
    /// Run one inspection; every completion path retires the owned connection.
    /// Acquisition resets inherited session state. No caller-owned
    /// transaction is accepted or rolled back by this API.
    pub async fn inspect<T, E>(
        pool: &PgPool,
        inspect: impl AsyncFnOnce(&mut PgReadOnlySql<'_>) -> Result<T, E>,
    ) -> Result<T, PgSnapshotError<E>> {
        Self::inspect_with_profile(pool, None, inspect).await
    }

    /// Establish session authority before BEGIN. Profile verification cannot
    /// take the inspection snapshot before the inspector acquires its locks.
    pub async fn inspect_profiled<T, E>(
        pool: &PgPool,
        profile: &crate::PgSessionProfile,
        inspect: impl AsyncFnOnce(&mut PgReadOnlySql<'_>) -> Result<T, E>,
    ) -> Result<T, PgSnapshotError<E>> {
        Self::inspect_with_profile(pool, Some(profile), inspect).await
    }

    async fn inspect_with_profile<T, E>(
        pool: &PgPool,
        profile: Option<&crate::PgSessionProfile>,
        inspect: impl AsyncFnOnce(&mut PgReadOnlySql<'_>) -> Result<T, E>,
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
            normalize(lease.connection_mut()).await?;
            if let Some(profile) = profile {
                profile.apply(lease.connection_mut()).await?;
            }
            command(
                lease.connection_mut(),
                "BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY",
            )
            .await?;
            if profile.is_none() {
                command(
                    lease.connection_mut(),
                    "SET LOCAL search_path = pg_catalog, pg_temp",
                )
                .await?;
            }
            // No SELECT before the inspector: it may lock authoritative objects
            // before its first query establishes the snapshot.
            ScopeSavepoint::begin(lease.connection_mut()).await
        }
        .await;
        let guard = setup.map_err(PgSnapshotError::Transaction)?;
        let result = inspect(&mut PgReadOnlySql {
            connection: lease.connection_mut(),
        })
        .await;
        match result {
            Ok(value) => {
                if let Some(profile) = profile {
                    profile
                        .verify(lease.connection_mut())
                        .await
                        .map_err(PgTransactionError::from)
                        .map_err(PgSnapshotError::Transaction)?;
                }
                guard
                    .release(lease.connection_mut())
                    .await
                    .map_err(PgSnapshotError::Transaction)?;
                validate(lease.connection_mut())
                    .await
                    .map_err(PgSnapshotError::Transaction)?;
                command(lease.connection_mut(), "ROLLBACK")
                    .await
                    .map_err(PgSnapshotError::Transaction)?;
                Ok(value)
            }
            Err(inspection) => {
                // The private guard proves this is still the original boundary,
                // including after an aborted statement. Bare ROLLBACK outside a
                // transaction only emits a warning and cannot prove this.
                let cleanup = async {
                    guard.rollback(lease.connection_mut()).await?;
                    if let Some(profile) = profile {
                        profile.verify(lease.connection_mut()).await?;
                    }
                    validate(lease.connection_mut()).await?;
                    command(lease.connection_mut(), "ROLLBACK").await
                }
                .await;
                match cleanup {
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

async fn validate(connection: &mut PgConnection) -> Result<(), PgTransactionError> {
    let (isolation, read_only): (String, String) = sqlx::query_as(
        "SELECT pg_catalog.current_setting('transaction_isolation'), \
         pg_catalog.current_setting('transaction_read_only')",
    )
    .fetch_one(connection)
    .await?;
    if isolation != "repeatable read" {
        return Err(PgTransactionError::TransactionIsolationChanged);
    }
    if read_only != "on" {
        return Err(PgTransactionError::TransactionAccessChanged);
    }
    Ok(())
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
