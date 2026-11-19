use crate::{
    PgLease, PgTransactionError,
    atomic::scope::{ScopeSavepoint, command, normalize},
};
mod bounded;
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
        let mut session = Inspection::begin(pool, profile)
            .await
            .map_err(PgSnapshotError::Transaction)?;
        match inspect(&mut session.sql()).await {
            Ok(value) => {
                session
                    .finish(End::Accepted)
                    .await
                    .map_err(PgSnapshotError::Transaction)?;
                Ok(value)
            }
            Err(inspection) => match session.finish(End::Rejected).await {
                Ok(()) => Err(PgSnapshotError::Inspection(inspection)),
                Err(cleanup) => Err(PgSnapshotError::Cleanup {
                    inspection,
                    cleanup,
                }),
            },
        }
    }
}

struct Inspection<'a> {
    lease: PgLease,
    guard: ScopeSavepoint,
    profile: Option<&'a crate::PgSessionProfile>,
}

enum End {
    Accepted,
    Rejected,
}

impl<'a> Inspection<'a> {
    async fn begin(
        pool: &PgPool,
        profile: Option<&'a crate::PgSessionProfile>,
    ) -> Result<Self, PgTransactionError> {
        let mut lease = PgLease {
            connection: Some(crate::acquire(pool).await?),
        };
        let connection = lease.connection_mut();
        normalize(connection).await?;
        if let Some(profile) = profile {
            profile.apply(connection).await?;
        }
        command(
            connection,
            "BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY",
        )
        .await?;
        if profile.is_none() {
            command(connection, "SET LOCAL search_path = pg_catalog, pg_temp").await?;
        }
        // The inspector may lock objects before its first query takes the snapshot.
        let guard = ScopeSavepoint::begin(connection).await?;
        Ok(Self {
            lease,
            guard,
            profile,
        })
    }

    fn sql(&mut self) -> PgReadOnlySql<'_> {
        PgReadOnlySql {
            connection: self.lease.connection_mut(),
        }
    }

    async fn finish(mut self, end: End) -> Result<(), PgTransactionError> {
        let connection = self.lease.connection_mut();
        // The private guard proves continuity and recovers an aborted statement
        // before revalidation on the rejection path.
        match end {
            End::Accepted => self.guard.release(connection).await?,
            End::Rejected => self.guard.rollback(connection).await?,
        }
        if let Some(profile) = self.profile {
            profile.verify(connection).await?;
        }
        validate(connection).await?;
        command(connection, "ROLLBACK").await
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
