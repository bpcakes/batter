//! Every attempt lease state against every event, the time-only transitions,
//! saturation, and the one backoff schedule.
use runlimit_core::{
    Delay, PolicyId, QuotaPeriod, ScopeId,
    attempts::{
        AdmitEvaluation, AttemptDenial, AttemptLease, AttemptOutcome, AttemptPolicy, AttemptState,
        CompleteEvaluation, FailedAttempts, LeaseDenial,
    },
};
use std::{num::NonZeroU32, time::Duration};

const INITIAL: u64 = 10;
const MAXIMUM: u64 = 40;
const QUIET: u64 = 100;
const LEASE: u64 = 20;
const TOKEN: u8 = 1;
const OTHER_TOKEN: u8 = 2;

fn ms(value: u64) -> Duration {
    Duration::from_millis(value)
}
fn period(value: u64) -> QuotaPeriod {
    QuotaPeriod::new(ms(value)).unwrap()
}
fn policy_of(initial: u64, maximum: u64, quiet: u64, lease: u64) -> AttemptPolicy {
    AttemptPolicy::new(
        PolicyId::new("auth").unwrap(),
        ScopeId::new("identifier").unwrap(),
        period(initial),
        period(maximum),
        period(quiet),
        period(lease),
    )
    .unwrap()
}
fn policy() -> AttemptPolicy {
    policy_of(INITIAL, MAXIMUM, QUIET, LEASE)
}
fn leased(failures: u32, until: u64) -> AttemptState<u8> {
    AttemptState::Leased(AttemptLease::new(TOKEN, failures, ms(until)))
}
fn failed(failures: u32, last_failure: u64, retry_at: u64) -> AttemptState<u8> {
    AttemptState::Failed(FailedAttempts::new(
        NonZeroU32::new(failures).unwrap(),
        ms(last_failure),
        ms(retry_at),
    ))
}
fn grant(state: AttemptState<u8>, now: u64) -> (u32, Duration) {
    match state.admit(&policy(), ms(now)) {
        AdmitEvaluation::Grant(pending) => (pending.failures(), pending.until()),
        AdmitEvaluation::Denied { denial, .. } => panic!("unexpected denial {denial:?}"),
    }
}
fn denial(state: AttemptState<u8>, now: u64) -> (AttemptState<u8>, LeaseDenial) {
    match state.admit(&policy(), ms(now)) {
        AdmitEvaluation::Denied { settled, denial } => (settled, denial),
        AdmitEvaluation::Grant(pending) => panic!("unexpected grant {pending:?}"),
    }
}
fn busy(remaining: u64) -> LeaseDenial {
    LeaseDenial::Busy {
        retry_after: Delay::new(ms(remaining)),
    }
}
fn backoff(remaining: u64) -> LeaseDenial {
    LeaseDenial::Backoff {
        retry_after: Delay::new(ms(remaining)),
    }
}
fn stale(state: AttemptState<u8>, now: u64, token: u8) -> AttemptState<u8> {
    for outcome in [
        AttemptOutcome::Success,
        AttemptOutcome::Failure,
        AttemptOutcome::Abandoned,
    ] {
        let CompleteEvaluation::Stale { settled } =
            state.complete(&policy(), ms(now), &token, outcome)
        else {
            panic!("{outcome:?} applied to a state without that live lease");
        };
        assert_eq!(settled, state.settle(&policy(), ms(now)));
    }
    state.settle(&policy(), ms(now))
}

#[test]
fn clear_admits_without_failures_and_has_nothing_to_complete() {
    let clear = AttemptState::<u8>::Clear;
    assert_eq!(clear.settle(&policy(), ms(u64::MAX)), AttemptState::Clear);
    assert_eq!(grant(clear, 1_000), (0, ms(1_000 + LEASE)));
    assert_eq!(stale(clear, 1_000, TOKEN), AttemptState::Clear);
}

#[test]
fn a_live_lease_is_busy_until_its_deadline_and_keeps_its_state() {
    let state = leased(2, 1_000);
    for (now, remaining) in [(0, 1_000), (990, 10), (999, 1)] {
        assert_eq!(state.settle(&policy(), ms(now)), state);
        assert_eq!(denial(state, now), (state, busy(remaining)));
    }
}

#[test]
fn only_the_holding_token_completes_a_live_lease() {
    let state = leased(2, 1_000);
    assert_eq!(stale(state, 999, OTHER_TOKEN), state);
    assert_eq!(
        state.complete(&policy(), ms(999), &TOKEN, AttemptOutcome::Success),
        CompleteEvaluation::Applied {
            state: AttemptState::Clear,
            completion: runlimit_core::attempts::AttemptCompletion::new(
                AttemptOutcome::Success,
                0,
                Delay::new(Duration::ZERO)
            )
            .unwrap(),
        }
    );
    for outcome in [AttemptOutcome::Failure, AttemptOutcome::Abandoned] {
        // The third consecutive failure waits the capped 40 ms from completion.
        assert_eq!(
            state.complete(&policy(), ms(999), &TOKEN, outcome),
            CompleteEvaluation::Applied {
                state: failed(3, 999, 999 + MAXIMUM),
                completion: runlimit_core::attempts::AttemptCompletion::new(
                    outcome,
                    3,
                    Delay::new(ms(MAXIMUM))
                )
                .unwrap(),
            }
        );
    }
}

#[test]
fn an_expired_lease_fails_at_its_deadline_however_late_it_is_noticed() {
    let state = leased(0, 1_000);
    // The deadline itself is expired, and the failure is dated there.
    for now in [1_000, 1_005, 1_009] {
        let counted = failed(1, 1_000, 1_000 + INITIAL);
        assert_eq!(state.settle(&policy(), ms(now)), counted);
        assert_eq!(denial(state, now), (counted, backoff(1_010 - now)));
    }
    // Past the delay the next reservation carries the counted failure forward.
    assert_eq!(grant(state, 1_010), (1, ms(1_010 + LEASE)));
    assert_eq!(grant(state, 1_099), (1, ms(1_099 + LEASE)));
    assert_eq!(
        grant(leased(1, 1_000), 1_020),
        (2, ms(1_020 + LEASE)),
        "an expired lease after one failure is the second failure"
    );
}

#[test]
fn completing_an_expired_lease_is_stale_and_counts_the_expiry_instead() {
    let state = leased(1, 1_000);
    // The receipt's own token no longer holds a lease: the expiry already
    // counted as the second failure, dated at the deadline, whatever outcome
    // the late completion carries.
    assert_eq!(stale(state, 1_000, TOKEN), failed(2, 1_000, 1_020));
    assert_eq!(stale(state, 1_050, TOKEN), failed(2, 1_000, 1_020));
    assert_eq!(stale(state, 1_100, TOKEN), AttemptState::Clear);
}

#[test]
fn the_quiet_period_runs_from_the_expiry_not_from_when_it_is_noticed() {
    let state = leased(3, 1_000);
    assert_eq!(
        state.settle(&policy(), ms(1_000 + QUIET - 1)),
        failed(4, 1_000, 1_000 + MAXIMUM)
    );
    assert_eq!(
        state.settle(&policy(), ms(1_000 + QUIET)),
        AttemptState::Clear
    );
    assert_eq!(
        grant(state, 1_000 + QUIET),
        (0, ms(1_000 + QUIET + LEASE)),
        "expiry then quiet reset: swapping the two would carry four failures"
    );
}

#[test]
fn failures_delay_until_retry_then_admit_and_clear_after_the_quiet_period() {
    let state = failed(2, 1_000, 1_020);
    for (now, remaining) in [(1_000, 20), (1_019, 1)] {
        assert_eq!(state.settle(&policy(), ms(now)), state);
        assert_eq!(denial(state, now), (state, backoff(remaining)));
    }
    assert_eq!(grant(state, 1_020), (2, ms(1_020 + LEASE)));
    assert_eq!(grant(state, 1_099), (2, ms(1_099 + LEASE)));
    assert_eq!(state.settle(&policy(), ms(1_100)), AttemptState::Clear);
    assert_eq!(grant(state, 1_100), (0, ms(1_100 + LEASE)));
    assert_eq!(stale(state, 1_010, TOKEN), state);
    assert_eq!(stale(state, 1_100, TOKEN), AttemptState::Clear);
}

#[test]
fn a_released_retry_time_admits_without_forgetting_failures() {
    // An operator may move the stored retry time before the last failure.
    let state = failed(3, 1_000, 0);
    assert_eq!(grant(state, 1_000), (3, ms(1_000 + LEASE)));
    let AdmitEvaluation::Grant(pending) = state.admit(&policy(), ms(1_000)) else {
        panic!("released subject was denied");
    };
    let CompleteEvaluation::Applied { completion, .. } = AttemptState::Leased(pending.issue(TOKEN))
        .complete(&policy(), ms(1_001), &TOKEN, AttemptOutcome::Failure)
    else {
        panic!("live lease was stale");
    };
    assert_eq!(completion.consecutive_failures(), 4);
}

#[test]
fn a_clock_behind_the_stored_instants_neither_expires_nor_clears_anything() {
    let lease = leased(1, 1_000);
    assert_eq!(denial(lease, 0), (lease, busy(1_000)));
    let failure = failed(1, 1_000, 1_010);
    assert_eq!(denial(failure, 0), (failure, backoff(1_010)));
}

#[test]
fn failure_counts_and_instants_saturate_instead_of_wrapping() {
    let policy = policy();
    let state = leased(u32::MAX, 1_000);
    assert_eq!(
        state.settle(&policy, ms(1_001)),
        failed(u32::MAX, 1_000, 1_000 + MAXIMUM)
    );
    let CompleteEvaluation::Applied { state, completion } =
        leased(u32::MAX - 1, 1_000).complete(&policy, ms(0), &TOKEN, AttemptOutcome::Failure)
    else {
        panic!("live lease was stale");
    };
    assert_eq!(state, failed(u32::MAX, 0, MAXIMUM));
    assert_eq!(completion.consecutive_failures(), u32::MAX);
    assert_eq!(completion.retry_after().duration(), ms(MAXIMUM));

    let end = Duration::MAX;
    let AdmitEvaluation::Grant(pending) = AttemptState::<u8>::Clear.admit(&policy, end) else {
        panic!("clear subject was denied");
    };
    assert_eq!(pending.until(), end);
    // At the end of representable time the failure's retry and quiet deadlines
    // both saturate to that same instant, so the expiry counts and then clears.
    let last = AttemptState::Leased(AttemptLease::new(TOKEN, 0, end));
    assert_eq!(last.settle(&policy, end), AttemptState::Clear);
    let late = AttemptState::<u8>::Failed(FailedAttempts::new(NonZeroU32::MIN, ms(1_000), end));
    assert_eq!(
        late.admit(&policy, ms(1_050)),
        AdmitEvaluation::Denied {
            settled: late,
            denial: LeaseDenial::Backoff {
                retry_after: Delay::new(end.saturating_sub(ms(1_050))),
            },
        }
    );
}

#[test]
fn settling_composes_over_time_for_every_state() {
    let policy = policy();
    let instants = [0, 1, 9, 10, 11, 19, 20, 21, 40, 99, 100, 101, 119, 120, 500];
    let mut states = vec![AttemptState::Clear];
    for failures in [0, 1, 2, 5, u32::MAX] {
        for at in [0, 10, 20, 100] {
            states.push(leased(failures, at));
            if failures > 0 {
                states.push(failed(failures, at, at + 10));
                states.push(failed(failures, at, 0));
            }
        }
    }
    for state in states {
        for (index, earlier) in instants.into_iter().enumerate() {
            let once = state.settle(&policy, ms(earlier));
            assert_eq!(once.settle(&policy, ms(earlier)), once, "{state:?}");
            for later in &instants[index..] {
                assert_eq!(
                    once.settle(&policy, ms(*later)),
                    state.settle(&policy, ms(*later)),
                    "{state:?} settled at {earlier} then {later}"
                );
                // A denial's settled state answers later admissions identically.
                if let AdmitEvaluation::Denied { settled, .. } = state.admit(&policy, ms(earlier)) {
                    assert_eq!(
                        settled.admit(&policy, ms(*later)),
                        state.admit(&policy, ms(*later))
                    );
                }
            }
        }
    }
}

#[test]
fn lease_denials_convert_to_the_matching_attempt_denials() {
    let delay = Delay::new(ms(7));
    assert_eq!(
        AttemptDenial::from(LeaseDenial::Busy { retry_after: delay }),
        AttemptDenial::Busy { retry_after: delay }
    );
    assert_eq!(
        AttemptDenial::from(LeaseDenial::Backoff { retry_after: delay }),
        AttemptDenial::Backoff { retry_after: delay }
    );
}

/// The schedule stated independently of `AttemptPolicy::delay`, in wide
/// integers: `min(maximum, initial * 2^(failures - 1))`.
fn reference_delay(initial: u64, maximum: u64, failures: u32) -> u128 {
    if failures == 0 {
        return 0;
    }
    let doublings = (failures - 1).min(64);
    (u128::from(initial) << doublings).min(u128::from(maximum))
}

#[test]
fn one_schedule_covers_every_failure_count_and_policy_bound() {
    let maximum = QuotaPeriod::MAX.millis();
    for (initial, cap) in [
        (INITIAL, MAXIMUM),
        (1, 1),
        (1, 2),
        (1, maximum),
        (3, 1_000),
        (7, 7),
        (maximum - 1, maximum),
        (maximum, maximum),
    ] {
        let policy = policy_of(initial, cap, cap, 1);
        let schedule = policy.delay_schedule();
        assert!(!schedule.is_empty() && schedule.len() <= 65);
        assert_eq!(schedule.last(), Some(&ms(cap)));
        assert!(schedule.windows(2).all(|pair| pair[0] < pair[1]));
        assert_eq!(policy.delay(0), Duration::ZERO);
        for failures in (1..=130).chain([u32::MAX - 1, u32::MAX]) {
            let expected = reference_delay(initial, cap, failures);
            assert_eq!(policy.delay(failures).as_millis(), expected);
            let tabulated = schedule[usize::try_from(failures).unwrap().min(schedule.len()) - 1];
            assert_eq!(
                tabulated.as_millis(),
                expected,
                "{initial} {cap} {failures}"
            );
        }
    }
}

#[test]
fn completed_and_expired_failures_take_the_same_delay() {
    let policy = policy();
    for prior in [0, 1, 2, 3, 9] {
        let state = leased(prior, 1_000);
        let CompleteEvaluation::Applied {
            state: completed,
            completion,
        } = state.complete(&policy, ms(500), &TOKEN, AttemptOutcome::Failure)
        else {
            panic!("live lease was stale");
        };
        let AttemptState::Failed(completed) = completed else {
            panic!("a failure did not leave failure state");
        };
        let AttemptState::Failed(expired) = state.settle(&policy, ms(1_000)) else {
            panic!("an expired lease did not count as a failure");
        };
        let delay = policy.delay(prior + 1);
        assert_eq!(completion.retry_after().duration(), delay);
        assert_eq!(completed.failures(), expired.failures());
        assert_eq!(
            completed.last_failure().saturating_add(delay),
            completed.retry_at()
        );
        assert_eq!(
            expired.last_failure().saturating_add(delay),
            expired.retry_at()
        );
    }
}
