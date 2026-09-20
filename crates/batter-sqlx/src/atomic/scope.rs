use super::PgTransactionError;
use sqlx::{Connection, Executor, PgConnection, Postgres};

/// Native SQL execution confined to a library-owned scope.
///
/// No connection reference, completion method or native transaction escapes.
/// Arbitrary SQL can have irreversible effects (including an explicit COMMIT).
/// The owner detects broken transaction boundaries before returning usable state;
/// this capability is not a sandbox against deliberate manipulation of the
/// library's internal savepoints or database administration.
///
/// ```compile_fail,E0616
/// fn forge(connection: &mut sqlx::PgConnection) {
///     let _ = batter_sqlx::PgScopedSql { connection };
/// }
/// ```
/// ```compile_fail,E0614
/// fn extract<'a>(sql: &'a mut batter_sqlx::PgScopedSql<'_>) -> &'a mut sqlx::PgConnection {
///     &mut **sql
/// }
/// ```
pub struct PgScopedSql<'a> {
    pub(crate) connection: &'a mut PgConnection,
}

impl PgScopedSql<'_> {
    /// Borrow the same retained physical connection for a native SQLx query.
    pub fn executor(&mut self) -> impl Executor<'_, Database = Postgres> {
        &mut *self.connection
    }
}

impl crate::PgExecutor for PgScopedSql<'_> {
    fn executor(&mut self) -> impl Executor<'_, Database = Postgres> {
        self.executor()
    }
}

pub(crate) async fn command(
    connection: &mut PgConnection,
    statement: &'static str,
) -> Result<(), PgTransactionError> {
    sqlx::raw_sql(statement).execute(connection).await?;
    Ok(())
}

// Reset inherited session resources before establishing any transaction evidence.
// Clear the driver's cache first: DISCARD ALL also deallocates server statements.
// Any failure leaves the owning lease armed for retirement.
pub(crate) async fn normalize(connection: &mut PgConnection) -> Result<(), PgTransactionError> {
    command(connection, "ROLLBACK").await?;
    connection.clear_cached_statements().await?;
    command(connection, "DISCARD ALL").await
}

// Identifier comes solely from a locally generated UUID. Never interpolate
// application input. A fresh name prevents collisions with ordinary SQL and
// releasing this parent also removes every application-created child savepoint.
pub(crate) struct ScopeSavepoint(String);

impl ScopeSavepoint {
    pub(crate) async fn begin(connection: &mut PgConnection) -> Result<Self, PgTransactionError> {
        let scope = Self(format!("batter_{}", uuid::Uuid::new_v4().simple()));
        scope.execute(connection, "SAVEPOINT").await?;
        Ok(scope)
    }

    pub(crate) async fn release(
        &self,
        connection: &mut PgConnection,
    ) -> Result<(), PgTransactionError> {
        self.execute(connection, "RELEASE SAVEPOINT").await
    }

    pub(crate) async fn rollback(
        &self,
        connection: &mut PgConnection,
    ) -> Result<(), PgTransactionError> {
        self.execute(connection, "ROLLBACK TO SAVEPOINT").await?;
        self.release(connection).await
    }

    async fn execute(
        &self,
        connection: &mut PgConnection,
        verb: &str,
    ) -> Result<(), PgTransactionError> {
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!("{verb} {}", self.0)))
            .execute(connection)
            .await?;
        Ok(())
    }
}
