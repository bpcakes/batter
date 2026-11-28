//! The only decode of a persisted attempt row, and the only conversions
//! between the table's millisecond columns and the shared lease state's time.
use runlimit_core::attempts::{
    AttemptLease, AttemptPolicy, AttemptState, FailedAttempts, StorageFenced,
};
use sqlx::{Row, postgres::PgRow};
use std::{num::NonZeroU32, time::Duration};

/// The lease columns of one `runlimit_attempts` row, exactly as stored.
#[derive(Clone, Copy, Debug)]
pub(crate) struct StoredAttempt {
    pub(super) failures: i64,
    pub(super) last_failure_ms: i64,
    pub(super) retry_at_ms: i64,
    pub(super) lease_until_ms: Option<i64>,
}

impl StoredAttempt {
    pub(super) fn read(row: &PgRow) -> Result<Self, sqlx::Error> {
        Ok(Self {
            failures: row.try_get("failures")?,
            last_failure_ms: row.try_get("last_failure_ms")?,
            retry_at_ms: row.try_get("retry_at_ms")?,
            lease_until_ms: row.try_get("lease_until_ms")?,
        })
    }

    /// Decodes the row into the one state it can mean, or names the reason no
    /// state fits. Lease tokens are compared by the database, so the state
    /// carries none and cannot evaluate a completion.
    ///
    /// While a lease is held the failure timestamps are unread: every exit
    /// from the lease overwrites both. Without a lease, zero failures mean
    /// [`AttemptState::Clear`], which has no delay to honor, so such a row with
    /// a retry time after its last activity is rejected instead of silently
    /// dropping or enforcing the delay.
    pub(super) fn state(self) -> Result<AttemptState<StorageFenced>, &'static str> {
        let failures = u32::try_from(self.failures).map_err(|_| "invalid attempt failure count")?;
        if let Some(until) = self.lease_until_ms {
            return Ok(AttemptState::Leased(AttemptLease::new(
                StorageFenced,
                failures,
                instant(until),
            )));
        }
        let Some(failures) = NonZeroU32::new(failures) else {
            if self.retry_at_ms > self.last_failure_ms {
                return Err("attempt without failures carries a retry delay");
            }
            return Ok(AttemptState::Clear);
        };
        Ok(AttemptState::Failed(FailedAttempts::new(
            failures,
            instant(self.last_failure_ms),
            instant(self.retry_at_ms),
        )))
    }
}

/// The shared state's time is unsigned and the table's instants are signed
/// `BIGINT` milliseconds, so the state's epoch is the earliest `BIGINT`. The
/// shift keeps every stored instant and the sampled clock in their stored
/// order with their exact differences, including instants before 1970.
const EPOCH_SHIFT: u64 = 1 << 63;

/// A stored or sampled millisecond instant in the shared state's time.
pub(super) fn instant(millis: i64) -> Duration {
    Duration::from_millis(millis.cast_unsigned() ^ EPOCH_SHIFT)
}

/// The inverse of [`instant`], or `None` for an instant after the last `BIGINT`.
pub(super) fn stored_instant(instant: Duration) -> Option<i64> {
    let millis = u64::try_from(instant.as_millis()).ok()?;
    Some((millis ^ EPOCH_SHIFT).cast_signed())
}

/// Whole milliseconds of a duration for a `BIGINT` value, or `None` beyond it.
fn millis(value: Duration) -> Option<i64> {
    i64::try_from(value.as_millis()).ok()
}

/// [`AttemptPolicy::delay_schedule`] in the completion statement's unit.
pub(super) fn delay_schedule(policy: &AttemptPolicy) -> Vec<i64> {
    policy
        .delay_schedule()
        .into_iter()
        .map(|delay| millis(delay).expect("a validated policy delay fits BIGINT"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{StoredAttempt, delay_schedule, instant, stored_instant};
    use runlimit_core::{
        PolicyId, QuotaPeriod, ScopeId,
        attempts::{
            AdmitEvaluation, AttemptLease, AttemptPolicy, AttemptState, FailedAttempts,
            LeaseDenial, StorageFenced,
        },
    };
    use std::{num::NonZeroU32, time::Duration};

    const fn row(failures: i64, last: i64, retry: i64, lease: Option<i64>) -> StoredAttempt {
        StoredAttempt {
            failures,
            last_failure_ms: last,
            retry_at_ms: retry,
            lease_until_ms: lease,
        }
    }
    /// The mapping stated independently of `instant`: milliseconds since the
    /// earliest `BIGINT`.
    fn at(millis: i64) -> Duration {
        let since_earliest = i128::from(millis) - i128::from(i64::MIN);
        Duration::from_millis(u64::try_from(since_earliest).unwrap())
    }
    fn failed(failures: u32, last: i64, retry: i64) -> AttemptState<StorageFenced> {
        AttemptState::Failed(FailedAttempts::new(
            NonZeroU32::new(failures).unwrap(),
            at(last),
            at(retry),
        ))
    }
    fn leased(failures: u32, until: i64) -> AttemptState<StorageFenced> {
        AttemptState::Leased(AttemptLease::new(StorageFenced, failures, at(until)))
    }

    #[test]
    fn every_written_row_shape_decodes_to_exactly_one_state() {
        // Success completion and a never-failed row.
        assert_eq!(row(0, 500, 500, None).state(), Ok(AttemptState::Clear));
        // Failure completion, and an operator release that keeps the failures.
        assert_eq!(row(3, 500, 540, None).state(), Ok(failed(3, 500, 540)));
        assert_eq!(row(3, 500, 0, None).state(), Ok(failed(3, 500, 0)));
        assert_eq!(
            row(i64::from(u32::MAX), 0, 0, None).state(),
            Ok(failed(u32::MAX, 0, 0))
        );
        // A lease ignores the failure timestamps, whatever they hold.
        assert_eq!(row(0, 500, 500, Some(900)).state(), Ok(leased(0, 900)));
        assert_eq!(row(2, -1, i64::MAX, Some(0)).state(), Ok(leased(2, 0)));
        // Instants before 1970 keep their exact value.
        assert_eq!(row(1, 500, 500, Some(-1)).state(), Ok(leased(1, -1)));
        assert_eq!(row(1, -7, -3, None).state(), Ok(failed(1, -7, -3)));
        assert_eq!(
            row(1, 500, i64::MIN, None).state(),
            Ok(failed(1, 500, i64::MIN))
        );
    }

    #[test]
    fn stored_instants_keep_their_order_and_differences_across_the_epoch() {
        let samples = [i64::MIN, -1_000, -1, 0, 1, 1_000, i64::MAX];
        for (index, earlier) in samples.into_iter().enumerate() {
            assert_eq!(instant(earlier), at(earlier));
            assert_eq!(stored_instant(instant(earlier)), Some(earlier));
            for later in &samples[index..] {
                assert_eq!(
                    instant(*later)
                        .checked_sub(instant(earlier))
                        .unwrap()
                        .as_millis(),
                    u128::try_from(i128::from(*later) - i128::from(earlier)).unwrap()
                );
            }
        }
        assert_eq!(
            stored_instant(instant(i64::MAX) + Duration::from_millis(1)),
            None
        );
    }

    /// A lease that ended before 1970 is settled from its real deadline, as the
    /// signed arithmetic it replaces did, not from the epoch.
    #[test]
    fn a_lease_that_ended_before_the_epoch_backs_off_from_its_own_deadline() {
        let period = |millis| QuotaPeriod::new(Duration::from_millis(millis)).unwrap();
        let policy = AttemptPolicy::new(
            PolicyId::new("auth").unwrap(),
            ScopeId::new("identifier").unwrap(),
            period(10),
            period(40),
            period(100),
            period(20),
        )
        .unwrap();
        // Expired at -20, so the first failure delays until -10 and stays
        // retained until 80.
        let state = row(0, 0, 0, Some(-20)).state().unwrap();
        let AdmitEvaluation::Denied(denied) = state.admit(&policy, instant(-15)) else {
            panic!("admitted inside the delay");
        };
        assert_eq!(
            denied.denial(),
            LeaseDenial::Backoff {
                retry_after: runlimit_core::Delay::new(Duration::from_millis(5)),
            }
        );
        let AdmitEvaluation::Grant(pending) = state.admit(&policy, instant(5)) else {
            panic!("denied after the delay ended at -10");
        };
        assert_eq!(pending.failures(), 1);
        assert_eq!(stored_instant(pending.until()), Some(25));
    }

    #[test]
    fn rows_that_fit_no_state_are_named_not_reinterpreted() {
        for failures in [-1, i64::from(u32::MAX) + 1, i64::MIN, i64::MAX] {
            for lease in [None, Some(900)] {
                assert_eq!(
                    row(failures, 500, 500, lease).state(),
                    Err("invalid attempt failure count")
                );
            }
        }
        assert_eq!(
            row(0, 500, 501, None).state(),
            Err("attempt without failures carries a retry delay")
        );
        assert_eq!(
            row(0, -2, -1, None).state(),
            Err("attempt without failures carries a retry delay")
        );
        assert_eq!(row(0, -1, -2, None).state(), Ok(AttemptState::Clear));
    }

    /// The completion statement reads `schedule[LEAST(failures + 1, cardinality)]`
    /// with one-based subscripts. This models that lookup and nothing else.
    fn statement_delay(schedule: &[i64], stored_failures: u32) -> i64 {
        let subscript = (u64::from(stored_failures) + 1).min(schedule.len() as u64);
        schedule[usize::try_from(subscript).unwrap() - 1]
    }

    #[test]
    fn the_bound_schedule_gives_the_statement_the_policy_delay_for_every_count() {
        let period = |millis| QuotaPeriod::new(Duration::from_millis(millis)).unwrap();
        let maximum = QuotaPeriod::MAX.millis();
        for (initial, cap) in [
            (10, 40),
            (1, 1),
            (1, maximum),
            (3, 1_000),
            (7, 7),
            (maximum, maximum),
            (maximum / 2 + 1, maximum),
        ] {
            let policy = AttemptPolicy::new(
                PolicyId::new("auth").unwrap(),
                ScopeId::new("identifier").unwrap(),
                period(initial),
                period(cap),
                period(cap),
                period(1),
            )
            .unwrap();
            let schedule = delay_schedule(&policy);
            assert!(!schedule.is_empty() && schedule.len() <= 65);
            for stored in (0..=130).chain([u32::MAX - 1, u32::MAX]) {
                let next = stored.saturating_add(1);
                assert_eq!(
                    u128::try_from(statement_delay(&schedule, stored)).unwrap(),
                    policy.delay(next).as_millis(),
                    "initial {initial} cap {cap} stored failures {stored}"
                );
            }
        }
    }
}
