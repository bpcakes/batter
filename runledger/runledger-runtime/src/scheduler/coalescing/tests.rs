use std::cell::Cell;
use std::str::FromStr;

use chrono::{DateTime, Duration, Utc};
use cron::Schedule;
use uuid::Uuid;

use super::super::{NextFire, next_fire_after_materialization};
use super::{
    CoalescedSchedule, DROPPED_OCCURRENCE_COUNT_CAP, DroppedOccurrences, count_dropped_occurrences,
};

fn utc(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .expect("fixed timestamp")
        .with_timezone(&Utc)
}

/// Builds the notice exactly as materialization would, or `None` on schedule.
fn coalesced(
    cron_expr: &str,
    scheduled_for: DateTime<Utc>,
    now: DateTime<Utc>,
    max_jitter_seconds: i32,
) -> Option<CoalescedSchedule> {
    let cron = Schedule::from_str(cron_expr).expect("cron expression must parse");
    let next =
        next_fire_after_materialization(&cron, scheduled_for, now, Uuid::nil(), max_jitter_seconds)
            .expect("schedule has a next fire");
    let NextFire::Coalesced(next_fire_at) = next else {
        return None;
    };
    Some(CoalescedSchedule {
        schedule_id: Uuid::nil(),
        schedule_name: "coalescing-fixture".to_owned(),
        cron,
        max_jitter_seconds,
        scheduled_for,
        now,
        next_fire_at,
    })
}

#[test]
fn outage_lag_counts_every_missed_occurrence_up_to_now() {
    let notice = coalesced(
        "0 * * * * * *",
        utc("2026-02-01T12:00:00Z"),
        utc("2026-02-01T12:05:00Z"),
        0,
    )
    .expect("a five-minute lag coalesces");

    assert_eq!(notice.next_fire_at, utc("2026-02-01T12:06:00Z"));
    assert_eq!(
        notice.dropped_occurrences(),
        DroppedOccurrences::Exactly(5),
        "12:01 through 12:05 are never materialized"
    );
}

#[test]
fn one_cron_period_of_lag_drops_one_occurrence_without_downtime() {
    let notice = coalesced(
        "0 * * * * * *",
        utc("2026-02-01T12:00:00Z"),
        utc("2026-02-01T12:01:00.200Z"),
        0,
    )
    .expect("a pass late by one period coalesces");

    assert_eq!(notice.next_fire_at, utc("2026-02-01T12:02:00Z"));
    assert_eq!(notice.dropped_occurrences(), DroppedOccurrences::Exactly(1));
}

#[test]
fn on_time_pass_does_not_coalesce() {
    assert!(
        coalesced(
            "0 * * * * * *",
            utc("2026-02-01T12:00:00Z"),
            utc("2026-02-01T12:00:59Z"),
            0,
        )
        .is_none(),
        "the next fire is still in the future, so nothing is dropped"
    );
}

#[test]
fn jittered_occurrence_chosen_as_the_next_fire_is_not_counted() {
    // Uuid::nil() with base 12:05:00 and 60s max jitter yields +54s, so the
    // cursor moves to a fire whose cron occurrence is not later than now.
    let notice = coalesced(
        "0 * * * * * *",
        utc("2026-02-01T12:00:00Z"),
        utc("2026-02-01T12:05:30Z"),
        60,
    )
    .expect("a five-minute lag coalesces");

    assert_eq!(notice.next_fire_at, utc("2026-02-01T12:05:54Z"));
    assert_eq!(
        notice.dropped_occurrences(),
        DroppedOccurrences::Exactly(4),
        "12:01 through 12:04 are dropped; 12:05 still fires at 12:05:54"
    );
}

#[test]
fn per_second_lag_after_a_year_is_capped_with_a_bounded_scan() {
    let now = utc("2026-02-01T12:00:00Z");
    let scheduled_for = now - Duration::days(365);
    let notice =
        coalesced("* * * * * * *", scheduled_for, now, 0).expect("a year of lag coalesces");
    assert_eq!(
        notice.dropped_occurrences(),
        DroppedOccurrences::AtLeast(DROPPED_OCCURRENCE_COUNT_CAP)
    );

    let scanned = Cell::new(0_usize);
    let candidates = notice
        .cron
        .after(&scheduled_for)
        .inspect(|_| scanned.set(scanned.get() + 1));
    let dropped = count_dropped_occurrences(candidates, now, |_| false);

    assert_eq!(
        dropped,
        DroppedOccurrences::AtLeast(DROPPED_OCCURRENCE_COUNT_CAP)
    );
    assert!(
        scanned.get() <= DROPPED_OCCURRENCE_COUNT_CAP + 2,
        "about 31.5 million missed seconds must not be iterated, scanned {}",
        scanned.get()
    );
}

#[test]
fn count_at_the_cap_is_exact_and_one_more_is_reported_as_at_least() {
    let now = utc("2026-02-01T12:00:00Z");
    let occurrences =
        |count: usize| (1..=count).map(move |offset| now - Duration::seconds(offset as i64));

    assert_eq!(
        count_dropped_occurrences(occurrences(DROPPED_OCCURRENCE_COUNT_CAP), now, |_| false),
        DroppedOccurrences::Exactly(DROPPED_OCCURRENCE_COUNT_CAP)
    );
    assert_eq!(
        count_dropped_occurrences(occurrences(DROPPED_OCCURRENCE_COUNT_CAP + 1), now, |_| {
            false
        }),
        DroppedOccurrences::AtLeast(DROPPED_OCCURRENCE_COUNT_CAP)
    );
}

#[test]
fn only_one_next_fire_occurrence_is_excluded_and_later_ones_stop_the_count() {
    let now = utc("2026-02-01T12:00:00Z");
    let candidates = [
        now - Duration::seconds(3),
        now - Duration::seconds(2),
        now,
        now + Duration::seconds(1),
    ];

    assert_eq!(
        count_dropped_occurrences(candidates.into_iter(), now, |_| true),
        DroppedOccurrences::Exactly(2),
        "one occurrence is the next fire; occurrences after now are not dropped"
    );
}
