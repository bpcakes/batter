//! One immediate-first cadence, with no lateness tolerance or catch-up queue.

use crate::lifecycle::{PeriodicAdmission, PeriodicGrant, StartupPhase};
use std::time::Duration;
use tokio::time::{Instant, sleep_until};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CadencePoint {
    At(Instant),
    /// The monotonic clock cannot represent another cadence point. The
    /// component stays alive and interruptible by its startup and shutdown
    /// boundaries without overflowing or spinning.
    Exhausted,
}

pub(super) struct Schedule {
    period: Duration,
    next: CadencePoint,
}

impl Schedule {
    pub(super) fn new(period: Duration) -> Self {
        Self {
            period,
            next: CadencePoint::At(Instant::now()),
        }
    }

    /// Waiting and admission are one operation: a tick is never delivered
    /// without asking the lifecycle for its grant.
    pub(super) async fn admit(
        &mut self,
        admission: &PeriodicAdmission,
        phase: StartupPhase,
    ) -> Option<PeriodicGrant> {
        self.tick().await;
        admission.admit(phase)
    }

    async fn tick(&mut self) {
        let CadencePoint::At(due) = self.next else {
            std::future::pending::<()>().await;
            return;
        };
        sleep_until(due).await;
        // Advance before admitting this run. An overdue tick is delivered once;
        // the next tick is strictly after now, on the original cadence. Unlike
        // Tokio Interval::Skip, this also skips lateness of five ms or less.
        self.next = match following(due, Instant::now(), self.period) {
            Some(next) => CadencePoint::At(next),
            None => CadencePoint::Exhausted,
        };
    }
}

fn following(due: Instant, now: Instant, period: Duration) -> Option<Instant> {
    let remainder = now.saturating_duration_since(due).as_nanos() % period.as_nanos();
    // The remainder is smaller than a Duration, so both parts fit exactly.
    let remainder = Duration::new(
        (remainder / 1_000_000_000) as u64,
        (remainder % 1_000_000_000) as u32,
    );
    now.checked_add(period - remainder)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn realignment_preserves_phase_for_short_and_large_periods() {
        let origin = Instant::now();
        for period in [
            Duration::from_nanos(1),
            Duration::from_micros(250),
            Duration::from_secs(31_536_000),
        ] {
            let now = origin + period * 3 + period / 2;
            assert_eq!(
                following(origin, now, period),
                origin.checked_add(period * 4)
            );
        }
    }

    #[test]
    fn elapsed_nanoseconds_do_not_narrow_to_u64() {
        let origin = Instant::now();
        let elapsed = Duration::from_secs(20_000_000_000);
        let period = Duration::from_nanos(3);
        let now = origin + elapsed;
        assert_eq!(
            following(origin, now, period),
            now.checked_add(Duration::from_nanos(1))
        );
    }

    #[tokio::test(start_paused = true)]
    async fn cancelling_a_wait_does_not_consume_its_tick() {
        let period = Duration::from_secs(1);
        let mut schedule = Schedule::new(period);
        schedule.tick().await;
        let next = schedule.next;
        tokio::select! {
            biased;
            () = schedule.tick() => panic!("the next tick is still in the future"),
            () = std::future::ready(()) => {}
        }
        assert_eq!(schedule.next, next);
        schedule.tick().await;
        assert_eq!(CadencePoint::At(Instant::now()), next);
    }
}
