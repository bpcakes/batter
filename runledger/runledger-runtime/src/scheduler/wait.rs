//! Cadence between scheduler passes.

use std::time::Duration;

use chrono::{DateTime, Utc};
use runledger_postgres::jobs;
use tracing::warn;

/// Shortest wait after a pass that did not fill its batch.
///
/// The earliest active fire can already be due after such a pass: another
/// scheduler may hold that row, or a write landed after the claim. Without a
/// floor the loop would re-poll without pause until that changes. 500 ms stays
/// below cron's one-second resolution, so a per-second schedule is still
/// claimed on its own tick, while a due row that this instance cannot claim
/// costs at most two cheap passes per second.
pub(super) const IDLE_WAIT_FLOOR: Duration = Duration::from_millis(500);

/// When the scheduler loop starts its next pass.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum NextPass {
    /// Yield, check shutdown, then claim the next batch.
    Immediately,
    /// Wait for shutdown or this long.
    After(Duration),
}

/// Whether a committed pass claimed a whole batch, so more rows may be due.
pub(super) fn batch_was_full(claimed: usize, batch_size: i64) -> bool {
    i64::try_from(claimed).is_ok_and(|claimed| claimed >= batch_size)
}

/// Reads the earliest active fire and returns how long to wait before the next
/// pass. The native helper caps the SQL statement at one second and awaits
/// rollback, so a table lock cannot indefinitely extend this post-commit phase.
/// A failed read falls back to the whole poll interval.
pub(super) async fn until_next_due_fire(
    pool: &runledger_postgres::DbPool,
    poll_interval: Duration,
) -> Duration {
    match jobs::earliest_active_schedule_fire_at(pool).await {
        Ok(earliest) => idle_wait(earliest, Utc::now(), poll_interval),
        Err(error) => {
            warn!(%error, "earliest schedule fire read failed; waiting the poll interval");
            poll_interval
        }
    }
}

/// `min(poll_interval, max(IDLE_WAIT_FLOOR, earliest - now))`, or the poll
/// interval when no schedule is active.
///
/// The cap keeps picking up schedules that other processes insert or move
/// earlier. A poll interval shorter than the floor still bounds the wait.
pub(super) fn idle_wait(
    earliest: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
    poll_interval: Duration,
) -> Duration {
    let Some(earliest) = earliest else {
        return poll_interval;
    };
    let until_due = (earliest - now).to_std().unwrap_or(Duration::ZERO);
    until_due.max(IDLE_WAIT_FLOOR).min(poll_interval)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use chrono::{DateTime, Duration as ChronoDuration, Utc};

    use super::{IDLE_WAIT_FLOOR, batch_was_full, idle_wait, until_next_due_fire};

    const POLL: Duration = Duration::from_secs(30);

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-05-26T12:00:00Z")
            .expect("fixed now")
            .with_timezone(&Utc)
    }

    #[test]
    fn floor_applies_when_the_earliest_fire_is_already_due() {
        for overdue in [ChronoDuration::zero(), ChronoDuration::hours(1)] {
            assert_eq!(
                idle_wait(Some(now() - overdue), now(), POLL),
                IDLE_WAIT_FLOOR,
                "a due but unclaimed row must not make the loop re-poll at once"
            );
        }
    }

    #[test]
    fn floor_applies_when_the_earliest_fire_is_sooner_than_the_floor() {
        let sooner = ChronoDuration::milliseconds(100);
        assert_eq!(
            idle_wait(Some(now() + sooner), now(), POLL),
            IDLE_WAIT_FLOOR
        );
    }

    #[test]
    fn wait_ends_at_the_earliest_fire_between_floor_and_cap() {
        let until_due = ChronoDuration::milliseconds(12_345);
        assert_eq!(
            idle_wait(Some(now() + until_due), now(), POLL),
            Duration::from_millis(12_345)
        );
    }

    #[test]
    fn poll_interval_caps_a_distant_earliest_fire() {
        let distant = ChronoDuration::days(365);
        assert_eq!(idle_wait(Some(now() + distant), now(), POLL), POLL);
    }

    #[test]
    fn poll_interval_applies_without_active_schedules() {
        assert_eq!(idle_wait(None, now(), POLL), POLL);
    }

    #[test]
    fn floor_never_lengthens_a_shorter_poll_interval() {
        let short_poll = Duration::from_millis(25);
        assert!(short_poll < IDLE_WAIT_FLOOR);
        assert_eq!(idle_wait(Some(now()), now(), short_poll), short_poll);
        assert_eq!(idle_wait(None, now(), short_poll), short_poll);
    }

    #[test]
    fn only_a_complete_batch_is_full() {
        assert!(batch_was_full(16, 16));
        assert!(!batch_was_full(15, 16));
        assert!(!batch_was_full(0, 1));
        assert!(batch_was_full(1, 1));
    }

    #[tokio::test]
    async fn failed_hint_preserves_the_exact_configured_interval() {
        // A closed native pool makes acquisition fail without network I/O.
        // Exercise the real hint failure boundary, including sub-floor and
        // non-round intervals, independently of loop scheduling tolerance.
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://localhost/hint_interval_test")
            .expect("valid lazy pool");
        pool.close().await;
        for configured in [Duration::from_millis(73), Duration::from_millis(37_123)] {
            assert_eq!(until_next_due_fire(&pool, configured).await, configured);
        }
    }
}
