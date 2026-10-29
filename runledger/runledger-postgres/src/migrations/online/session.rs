use sqlx::PgConnection;

/// The session's profile timeouts, saved while a concurrent build runs
/// without them and restored afterwards so whatever follows on that session
/// runs under the profile again: the cutover validation in the protected
/// runner, or the next build in external preparation.
///
/// A concurrent build waits for in-flight writers and older snapshots, so a
/// profile timeout would cancel it and leave an `INVALID` index behind. In
/// the protected runner the migration is already recorded by then, so only a
/// later startup could replace that index. While these bounds are lifted,
/// PostgreSQL checks for a disconnected client at most one second apart
/// (preserving a tighter configured check). This detects a closed socket on
/// supported Unix servers; it does not bound network failure detection or
/// promise backend death when the client runtime itself disappears.
pub(super) struct ProfileTimeouts {
    statement: String,
    lock: String,
    transaction: Option<String>,
    connection_check: String,
}

impl ProfileTimeouts {
    pub(super) async fn lift(conn: &mut PgConnection) -> Result<Self, sqlx::Error> {
        // transaction_timeout exists from PostgreSQL 17; read and lift it only
        // where the server knows it so an older server is not rejected after
        // the migration was recorded.
        let (statement, lock, transaction, connection_check): (
            String,
            String,
            Option<String>,
            String,
        ) = sqlx::query_as(
            "SELECT pg_catalog.current_setting('statement_timeout'),
                    pg_catalog.current_setting('lock_timeout'),
                    pg_catalog.current_setting('transaction_timeout', true),
                    pg_catalog.current_setting('client_connection_check_interval')",
        )
        .fetch_one(&mut *conn)
        .await?;
        sqlx::raw_sql(
            "SELECT pg_catalog.set_config('statement_timeout', '0', false),
                    pg_catalog.set_config('lock_timeout', '0', false),
                    CASE WHEN pg_catalog.current_setting('transaction_timeout', true) IS NOT NULL
                         THEN pg_catalog.set_config('transaction_timeout', '0', false)
                    END,
                    pg_catalog.set_config('client_connection_check_interval',
                        CASE WHEN pg_catalog.current_setting('client_connection_check_interval')::interval
                                       > interval '0'
                                  AND pg_catalog.current_setting('client_connection_check_interval')::interval
                                       < interval '1 second'
                             THEN pg_catalog.current_setting('client_connection_check_interval')
                             ELSE '1s' END, false)",
        )
        .execute(&mut *conn)
        .await?;
        Ok(Self {
            statement,
            lock,
            transaction,
            connection_check,
        })
    }

    pub(super) async fn restore(self, conn: &mut PgConnection) -> Result<(), sqlx::Error> {
        sqlx::query(
            "SELECT pg_catalog.set_config('statement_timeout', $1, false),
                    pg_catalog.set_config('lock_timeout', $2, false),
                    CASE WHEN $3::text IS NOT NULL
                         THEN pg_catalog.set_config('transaction_timeout', $3, false)
                    END,
                    pg_catalog.set_config('client_connection_check_interval', $4, false)",
        )
        .bind(self.statement)
        .bind(self.lock)
        .bind(self.transaction)
        .bind(self.connection_check)
        .execute(&mut *conn)
        .await?;
        Ok(())
    }
}
