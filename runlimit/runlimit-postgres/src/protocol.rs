use std::time::Duration;

use runlimit_core::CounterKey;
use sha2::{Digest, Sha256};
use sqlx::Error;
use tokio::time::Instant;

const ADVISORY_LOCK_DOMAIN: &[u8] = b"runlimit/postgres-advisory-lock/v1\0";
/// The two-integer advisory-lock namespace of attempt capacity shards; the
/// shard from [`capacity_shard`] is the second key. A persistent
/// cross-replica protocol: never change it in place.
pub(crate) const ATTEMPT_LOCK_NAMESPACE: i32 = 0x524c_4154;
pub(crate) const SERVER_TIMEOUT_GRACE: Duration = Duration::from_millis(25);
/// Number of stable capacity shards used by `PostgreSQL` storage.
///
/// The shard derivation is a persistent cross-replica protocol and this value
/// must not change in place.
pub const CAPACITY_SHARD_COUNT: usize = 256;

/// Database-enforced maximum number of fixed-window rows in one capacity
/// shard.
///
/// The additive cardinality migration enforces this ceiling independently of
/// the runtime configuration, including for replicas running older code.
pub const HARD_MAX_ROWS_PER_SHARD: u32 = 65_536;

pub(crate) fn is_server_timeout(error: &Error) -> bool {
    let sqlx::Error::Database(database_error) = error else {
        return false;
    };
    matches!(database_error.code().as_deref(), Some("57014" | "55P03"))
}

/// Returns the server statement and lock timeouts to set for the remaining
/// budget, or `None` when no budget remains.
///
/// A deadline that has been reached leaves nothing to spend, exactly like one
/// that has passed: the caller must fail before sending SQL.
///
/// The statement timeout is the remaining budget less
/// [`SERVER_TIMEOUT_GRACE`], and the lock timeout is 1 ms shorter. Both have a
/// 1 ms floor, because `PostgreSQL` reads zero as no timeout. With no more
/// than the grace plus 1 ms remaining, both settings sit at that floor. The
/// floor is longer than the remaining budget only when less than 1 ms remains,
/// and then the client deadline is the bound that holds.
pub(crate) fn remaining_server_timeout_settings(deadline: Instant) -> Option<(String, String)> {
    let remaining = deadline
        .checked_duration_since(Instant::now())
        .filter(|remaining| !remaining.is_zero())?;
    let remaining_millis = remaining.as_millis();
    let statement_millis = remaining_millis
        .saturating_sub(SERVER_TIMEOUT_GRACE.as_millis())
        .max(1);
    let lock_millis = statement_millis.saturating_sub(1).max(1);
    Some((format!("{statement_millis}ms"), format!("{lock_millis}ms")))
}

pub(crate) fn advisory_lock_id(counter_key: CounterKey) -> i64 {
    let mut digest = Sha256::new();
    digest.update(ADVISORY_LOCK_DOMAIN);
    digest.update(counter_key.to_bytes());
    let digest = digest.finalize();
    let mut id_bytes = [0_u8; 8];
    let id_length = id_bytes.len();
    id_bytes.copy_from_slice(&digest[..id_length]);
    i64::from_be_bytes(id_bytes)
}

/// The capacity shard of one stored row, for every storage family.
///
/// Fixed-window, GCRA and attempt rows all derive their shard here, from the
/// first byte of the policy fingerprint and of the subject key; each table's
/// generated `capacity_shard` column computes the same value. This is a
/// persistent cross-replica protocol: advisory locks, occupancy counts and
/// slot searches target the shard it names.
pub(crate) fn capacity_shard(counter_key: CounterKey) -> u8 {
    counter_key.fingerprint().as_bytes()[0] ^ counter_key.subject().as_bytes()[0]
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::time::Instant;

    use runlimit_core::{KeyHasher, PolicyId, QuotaPeriod, ScopeId, attempts::AttemptPolicy};

    use super::{SERVER_TIMEOUT_GRACE, capacity_shard, remaining_server_timeout_settings};
    use crate::statements::fixed_window::CLEANUP_SQL;

    // The attempt-protocol vectors in runlimit-core pin this policy's
    // fingerprint (first byte 0x38) and this subject's key (first byte 0xc6),
    // so the attempt row's generated `capacity_shard` column is 0x38 ^ 0xc6.
    #[test]
    fn attempt_capacity_shard_protocol_has_a_golden_vector() {
        let millis = |value| QuotaPeriod::new(Duration::from_millis(value)).unwrap();
        let policy = AttemptPolicy::new(
            PolicyId::new("sign-in").unwrap(),
            ScopeId::new("account").unwrap(),
            millis(1_000),
            millis(60_000),
            millis(600_000),
            millis(30_000),
        )
        .unwrap();
        let subject = KeyHasher::new([0x42; 32])
            .unwrap()
            .hash_attempt_for(&policy, b"user@example.test");

        assert_eq!(capacity_shard(subject.counter_key()), 254);
    }

    // The clock is paused, so "now" is the same instant on both sides of each
    // comparison and the reached-deadline case is exact.
    #[tokio::test(start_paused = true)]
    async fn a_reached_or_passed_deadline_leaves_no_server_timeout_to_set() {
        let now = Instant::now();

        assert_eq!(remaining_server_timeout_settings(now), None);
        tokio::time::advance(Duration::from_millis(1)).await;
        assert_eq!(remaining_server_timeout_settings(now), None);
    }

    #[tokio::test(start_paused = true)]
    async fn a_remaining_budget_sets_server_timeouts_with_a_one_millisecond_floor() {
        let now = Instant::now();
        let timeouts = |budget| remaining_server_timeout_settings(now + budget);

        // The floor is longer than a budget below 1 ms, which only the
        // client deadline bounds.
        assert_eq!(
            timeouts(Duration::from_nanos(1)),
            Some(("1ms".to_owned(), "1ms".to_owned()))
        );
        // A budget equal to the grace is still at the floor, now shorter than
        // the budget.
        assert_eq!(
            timeouts(SERVER_TIMEOUT_GRACE),
            Some(("1ms".to_owned(), "1ms".to_owned()))
        );
        // A larger budget leaves the grace for the client to observe the
        // server timeout, and the lock timeout fires before the statement's.
        assert_eq!(
            timeouts(Duration::from_secs(1)),
            Some(("975ms".to_owned(), "974ms".to_owned()))
        );
    }

    #[test]
    fn cleanup_query_identifies_rows_without_system_columns() {
        // A system-column read needs relation-level SELECT, which would let a
        // cleanup login read every counter column. The primary key is grantable
        // column by column.
        for system_column in ["ctid", "xmin", "xmax", "cmin", "cmax", "tableoid"] {
            assert!(
                !CLEANUP_SQL.sql().contains(system_column),
                "cleanup reads the {system_column} system column"
            );
        }
        assert!(
            CLEANUP_SQL
                .sql()
                .contains("windows.config_fingerprint = expired.config_fingerprint")
        );
        assert!(
            CLEANUP_SQL
                .sql()
                .contains("AND windows.subject_key = expired.subject_key")
        );
    }
}
