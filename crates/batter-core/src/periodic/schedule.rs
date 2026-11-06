//! One immediate-first cadence, optionally woken under a spacing floor.

use super::wake::WakeOwner;
use crate::lifecycle::{PeriodicAdmission, PeriodicGrant, StartupPhase};
use std::time::Duration;
use tokio::time::{Instant, sleep_until};

pub(super) enum Scheduling {
    Cadence,
    Wakeable { owner: WakeOwner, spacing: Duration },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CadencePoint {
    At(Instant),
    Exhausted,
}

// Spacing is tied to actual admission, never to wake arrival or completion.
enum SpacingGate {
    Open,
    Until(Instant),
    Exhausted,
}

enum Mode {
    Cadence,
    Wakeable {
        owner: WakeOwner,
        spacing: Duration,
        gate: SpacingGate,
    },
}

pub(super) struct Schedule {
    period: Duration,
    next: CadencePoint,
    mode: Mode,
}

impl Schedule {
    pub(super) fn new(period: Duration, scheduling: Scheduling) -> Self {
        Self {
            period,
            next: CadencePoint::At(Instant::now()),
            mode: match scheduling {
                Scheduling::Cadence => Mode::Cadence,
                Scheduling::Wakeable { owner, spacing } => Mode::Wakeable {
                    owner,
                    spacing,
                    gate: SpacingGate::Open,
                },
            },
        }
    }

    /// Waiting, admission and spacing advancement are one operation: callers
    /// cannot consume a wake without the lifecycle grant or forget the floor.
    pub(super) async fn admit(
        &mut self,
        admission: &PeriodicAdmission,
        phase: StartupPhase,
    ) -> Option<PeriodicGrant> {
        self.tick().await;
        let grant = admission.admit(phase)?;
        self.admitted();
        Some(grant)
    }

    async fn tick(&mut self) {
        if let Mode::Wakeable { gate, .. } = &self.mode {
            match gate {
                SpacingGate::Open => {}
                SpacingGate::Until(at) => sleep_until(*at).await,
                SpacingGate::Exhausted => std::future::pending().await,
            }
        }
        let cadence = async {
            match self.next {
                CadencePoint::At(at) => sleep_until(at).await,
                CadencePoint::Exhausted => std::future::pending().await,
            }
        };
        let wake = async {
            match &self.mode {
                Mode::Cadence => std::future::pending().await,
                Mode::Wakeable { owner, .. } => owner.pending().await,
            }
        };
        tokio::select! {
            biased;
            () = cadence => {},
            () = wake => {},
        }
        // Do not move the original cadence for an early wake. A tick deferred
        // by spacing remains due until this point, then realigns from its
        // original phase. All missed ticks coalesce, including sub-five-ms ones.
        if let CadencePoint::At(due) = self.next {
            let now = Instant::now();
            if due <= now {
                self.next = match following(due, now, self.period) {
                    Some(next) => CadencePoint::At(next),
                    None => CadencePoint::Exhausted,
                };
            }
        }
    }

    fn admitted(&mut self) {
        match &mut self.mode {
            Mode::Cadence => {}
            Mode::Wakeable {
                owner,
                spacing,
                gate,
            } => {
                owner.consume();
                *gate = match Instant::now().checked_add(*spacing) {
                    Some(at) => SpacingGate::Until(at),
                    None => SpacingGate::Exhausted,
                };
            }
        }
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
        let mut schedule = Schedule::new(period, Scheduling::Cadence);
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
