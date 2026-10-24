//! Session-scoped acquisition of SQLx's migration advisory lock that holds no
//! snapshot while waiting.
//!
//! SQLx's `Migrate::lock` blocks inside `SELECT pg_advisory_lock($1)`. That
//! statement holds a snapshot for as long as it waits, and a concurrent index
//! build on the lock holder's session waits for every older snapshot before
//! it can finish, so the two sessions deadlock and PostgreSQL aborts one of
//! them. Polling `pg_try_advisory_lock` as separate statements leaves the
//! waiting session idle between polls, with no snapshot for the build to wait
//! on, while the same key still excludes every SQLx migrator.

use std::time::Duration;

use sqlx::PgConnection;

use crate::PgQueryExecutor;

/// SQLx's lock-id factor from `sqlx-postgres`'s migration lock derivation.
const LOCK_ID_FACTOR: i64 = 0x3d32_ad9e;
const POLL_INTERVAL: Duration = Duration::from_millis(100);

/// The SQLx migration advisory lock, held by one session until released.
///
/// Dropping the guard does not release the lock; the session owns it. Callers
/// keep the pooled connection on `close_on_drop`, so a cancelled future closes
/// the session instead of returning a locked one to the pool.
#[must_use = "the lock is held by the session until released"]
pub(in crate::migrations) struct MigrationLock {
    id: i64,
}

impl MigrationLock {
    /// Wait for the lock by polling, holding no snapshot between polls.
    ///
    /// The session's `statement_timeout`, `lock_timeout` and, where the
    /// server defines it, `transaction_timeout` bound the wait exactly as
    /// they bounded the single blocking `pg_advisory_lock` statement: the
    /// smallest nonzero value fails the wait once it elapses, and only when
    /// all of them are zero does the wait continue without bound. An expired
    /// bound is reported as [`sqlx::Error::Protocol`] naming the setting, not
    /// as the server's `55P03` or `57014` database error or a connection
    /// terminated by `transaction_timeout`, because each short poll statement
    /// completes within any timeout.
    pub(in crate::migrations) async fn acquire(
        conn: &mut PgConnection,
    ) -> Result<Self, sqlx::Error> {
        // transaction_timeout exists from PostgreSQL 17; a server without it
        // yields NULL here and contributes no bound.
        let (database_name, statement_timeout_ms, lock_timeout_ms, transaction_timeout_ms): (
            String,
            i64,
            i64,
            Option<i64>,
        ) = sqlx::query_as(
            "SELECT pg_catalog.current_database(),
                    (EXTRACT(EPOCH FROM pg_catalog.current_setting('statement_timeout')::interval)
                     * 1000)::bigint,
                    (EXTRACT(EPOCH FROM pg_catalog.current_setting('lock_timeout')::interval)
                     * 1000)::bigint,
                    (EXTRACT(EPOCH FROM pg_catalog.current_setting('transaction_timeout', true)::interval)
                     * 1000)::bigint",
        )
        .fetch_one(&mut *conn)
        .await?;
        let id = lock_id(&database_name);
        let bound = wait_bound(
            statement_timeout_ms,
            lock_timeout_ms,
            transaction_timeout_ms,
        );
        let started = tokio::time::Instant::now();
        loop {
            let acquired: bool = sqlx::query_scalar("SELECT pg_catalog.pg_try_advisory_lock($1)")
                .bind(id)
                .fetch_one(&mut *conn)
                .await?;
            if acquired {
                return Ok(Self { id });
            }
            let remaining = match bound {
                Some((bound, setting)) => bound.checked_sub(started.elapsed()).ok_or_else(|| {
                    sqlx::Error::Protocol(format!(
                        "the migration advisory lock was not acquired within the session {setting} of {} ms",
                        bound.as_millis()
                    ))
                })?,
                None => POLL_INTERVAL,
            };
            tokio::time::sleep(remaining.min(POLL_INTERVAL)).await;
        }
    }

    /// Whether another session currently holds the migration lock, which
    /// means a Runledger migration or preparation is in progress there.
    /// `pg_locks` is readable by every role, unlike per-backend progress.
    pub(in crate::migrations) async fn held_by_another_session(
        conn: &mut impl PgQueryExecutor,
    ) -> Result<bool, sqlx::Error> {
        let database_name: String = sqlx::query_scalar("SELECT pg_catalog.current_database()")
            .fetch_one(conn.executor())
            .await?;
        let id = lock_id(&database_name);
        // A bigint advisory lock is keyed by its high and low 32 bits.
        let class_id = (id >> 32) & 0xFFFF_FFFF;
        let object_id = id & 0xFFFF_FFFF;
        sqlx::query_scalar(
            "SELECT EXISTS (
                SELECT 1
                FROM pg_catalog.pg_locks l
                WHERE l.locktype = 'advisory'
                  AND l.database = (
                      SELECT oid FROM pg_catalog.pg_database
                      WHERE datname = pg_catalog.current_database()
                  )
                  AND l.classid::bigint = $1
                  AND l.objid::bigint = $2
                  AND l.objsubid = 1
                  AND l.granted
                  AND l.pid <> pg_catalog.pg_backend_pid()
             )",
        )
        .bind(class_id)
        .bind(object_id)
        .fetch_one(conn.executor())
        .await
    }

    /// Release the lock on the session that acquired it.
    pub(in crate::migrations) async fn release(
        self,
        conn: &mut PgConnection,
    ) -> Result<(), sqlx::Error> {
        let released: bool = sqlx::query_scalar("SELECT pg_catalog.pg_advisory_unlock($1)")
            .bind(self.id)
            .fetch_one(conn)
            .await?;
        if released {
            Ok(())
        } else {
            Err(sqlx::Error::Protocol(
                "the migration advisory lock was not held by this session at release".into(),
            ))
        }
    }
}

/// The bound a blocking lock statement would have had: the smallest nonzero
/// session timeout, named for the error message, or none when all are zero.
/// `transaction_timeout` is `None` on a server that does not define it.
///
/// PostgreSQL ignores a narrower timeout equal to a wider one, so an equal
/// value is attributed to the wider-scoped setting: the transaction before
/// the statement, the statement before its lock wait.
fn wait_bound(
    statement_timeout_ms: i64,
    lock_timeout_ms: i64,
    transaction_timeout_ms: Option<i64>,
) -> Option<(Duration, &'static str)> {
    let nonzero = |ms: i64| u64::try_from(ms).ok().filter(|ms| *ms > 0);
    [
        (
            "transaction_timeout",
            transaction_timeout_ms.and_then(nonzero),
        ),
        ("statement_timeout", nonzero(statement_timeout_ms)),
        ("lock_timeout", nonzero(lock_timeout_ms)),
    ]
    .into_iter()
    .filter_map(|(setting, ms)| ms.map(|ms| (ms, setting)))
    .min_by_key(|(ms, _)| *ms)
    .map(|(ms, setting)| (Duration::from_millis(ms), setting))
}

/// The lock id SQLx derives for a database name: `0x3d32ad9e` times the
/// CRC-32/ISO-HDLC checksum of its bytes. The product always fits in `i64`.
fn lock_id(database_name: &str) -> i64 {
    LOCK_ID_FACTOR * i64::from(crc32_iso_hdlc(database_name.as_bytes()))
}

fn crc32_iso_hdlc(bytes: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_matches_the_iso_hdlc_check_value() {
        assert_eq!(crc32_iso_hdlc(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32_iso_hdlc(b""), 0);
    }

    #[test]
    fn wait_bound_is_the_smallest_nonzero_session_timeout() {
        let bound = |ms, setting| Some((Duration::from_millis(ms), setting));
        assert_eq!(wait_bound(0, 0, None), None);
        assert_eq!(wait_bound(0, 0, Some(0)), None);
        assert_eq!(wait_bound(2000, 250, None), bound(250, "lock_timeout"));
        assert_eq!(wait_bound(500, 0, Some(0)), bound(500, "statement_timeout"));
        assert_eq!(wait_bound(0, 750, None), bound(750, "lock_timeout"));
        assert_eq!(wait_bound(300, 900, None), bound(300, "statement_timeout"));
        // transaction_timeout alone, shortest of three, and longer than another.
        assert_eq!(
            wait_bound(0, 0, Some(400)),
            bound(400, "transaction_timeout")
        );
        assert_eq!(
            wait_bound(2000, 250, Some(100)),
            bound(100, "transaction_timeout")
        );
        assert_eq!(wait_bound(2000, 250, Some(900)), bound(250, "lock_timeout"));
        assert_eq!(wait_bound(0, 600, Some(900)), bound(600, "lock_timeout"));
        // Equal values belong to the wider-scoped setting.
        assert_eq!(
            wait_bound(500, 0, Some(500)),
            bound(500, "transaction_timeout")
        );
        assert_eq!(wait_bound(500, 500, None), bound(500, "statement_timeout"));
        assert_eq!(wait_bound(-1, -1, Some(-1)), None);
    }

    #[test]
    fn lock_id_applies_the_sqlx_factor_without_overflow() {
        assert_eq!(lock_id("123456789"), 0x3d32_ad9e * 0xCBF4_3926_i64);
        assert_eq!(lock_id(""), 0);
        assert!(LOCK_ID_FACTOR.checked_mul(i64::from(u32::MAX)).is_some());
    }
}
