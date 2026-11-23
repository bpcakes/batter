//! Bounded diagnostics for catch-up coalescing.

use chrono::{DateTime, Utc};
use cron::Schedule;
use runledger_postgres::jobs::JobScheduleRecord;
use tracing::warn;
use uuid::Uuid;

/// Most dropped occurrences counted for one warning. Counting stops here and
/// the warning reports "at least" this many, so a per-second cron after a long
/// outage costs a bounded scan instead of one step per missed second.
pub(super) const DROPPED_OCCURRENCE_COUNT_CAP: usize = 1_000;

/// Cron occurrences that a coalesced cursor will never materialize.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum DroppedOccurrences {
    Exactly(usize),
    AtLeast(usize),
}

/// One coalesced materialization, reported after its transaction commits.
#[derive(Debug)]
pub(super) struct CoalescedSchedule {
    schedule_id: Uuid,
    schedule_name: String,
    cron: Schedule,
    max_jitter_seconds: i32,
    scheduled_for: DateTime<Utc>,
    now: DateTime<Utc>,
    next_fire_at: DateTime<Utc>,
}

impl CoalescedSchedule {
    pub(super) fn new(
        schedule: &JobScheduleRecord,
        cron: Schedule,
        now: DateTime<Utc>,
        next_fire_at: DateTime<Utc>,
    ) -> Self {
        Self {
            schedule_id: schedule.id,
            schedule_name: schedule.name.clone(),
            cron,
            max_jitter_seconds: schedule.max_jitter_seconds,
            scheduled_for: schedule.next_fire_at,
            now,
            next_fire_at,
        }
    }

    pub(super) fn dropped_occurrences(&self) -> DroppedOccurrences {
        count_dropped_occurrences(self.cron.after(&self.scheduled_for), self.now, |base| {
            super::apply_schedule_jitter(self.schedule_id, base, self.max_jitter_seconds)
                == self.next_fire_at
        })
    }

    pub(super) fn warn(&self) {
        let (dropped_occurrences, dropped_occurrences_capped) = match self.dropped_occurrences() {
            DroppedOccurrences::Exactly(0) => return,
            DroppedOccurrences::Exactly(count) => (count, false),
            DroppedOccurrences::AtLeast(count) => (count, true),
        };
        warn!(
            schedule_id = %self.schedule_id,
            schedule_name = %self.schedule_name,
            scheduled_for = %self.scheduled_for,
            next_fire_at = %self.next_fire_at,
            dropped_occurrences,
            dropped_occurrences_capped,
            "scheduler lagged by at least one cron period; coalescing dropped missed occurrences"
        );
    }
}

/// Counts cron occurrences strictly after the materialized `scheduled_for` and
/// not later than `now`, the pass clock, given as `candidates` from
/// `cron.after(&scheduled_for)`.
///
/// With jitter, one such occurrence can be the future fire the cursor moved
/// to; `is_next_fire` identifies it so it is not counted. Consumes at most
/// [`DROPPED_OCCURRENCE_COUNT_CAP`] + 2 candidates.
pub(super) fn count_dropped_occurrences(
    candidates: impl Iterator<Item = DateTime<Utc>>,
    now: DateTime<Utc>,
    mut is_next_fire: impl FnMut(DateTime<Utc>) -> bool,
) -> DroppedOccurrences {
    let mut count = 0;
    let mut next_fire_seen = false;
    for base in candidates.take_while(|base| *base <= now) {
        if !next_fire_seen && is_next_fire(base) {
            next_fire_seen = true;
            continue;
        }
        if count == DROPPED_OCCURRENCE_COUNT_CAP {
            return DroppedOccurrences::AtLeast(count);
        }
        count += 1;
    }
    DroppedOccurrences::Exactly(count)
}

#[cfg(test)]
mod delivery_tests;
#[cfg(test)]
mod tests;
