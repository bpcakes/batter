use super::*;

#[test]
fn panic_before_the_next_poll_cannot_reset_an_on_time_observation_window() {
    let started = Instant::now();
    let drain_after = Duration::from_secs(1);
    let mut output = Output::default();
    output.record(b"batter-fixture:drain-requested\n", started + drain_after);
    output.record(
        b"thread 'fixture' panicked at deliberate request failure\n",
        started + Duration::from_secs(3),
    );
    for polled_after in [Duration::from_secs(4), Duration::from_secs(6)] {
        assert_eq!(
            WaitPolicy::for_scenario("blocked-request-failure")
                .deadline(&output, started, started + polled_after)
                .unwrap(),
            Some(drain_after + OBSERVATION_LIMIT)
        );
    }
    let error = output.validate(&["drain-requested"], &[]).unwrap_err();
    assert!(error.contains("child panicked"), "{error}");
}

#[test]
fn delayed_startup_keeps_its_full_observation_even_when_polled_late() {
    let started = Instant::now();
    // Exercise the real policy with exact instants, including its boundary.
    // No OS sleep or runtime startup must fit inside a one-second test window.
    for elapsed in [
        Duration::from_secs(4),
        STARTUP_LIMIT - Duration::from_nanos(1),
        STARTUP_LIMIT,
    ] {
        let mut output = Output::default();
        output.record(b"batter-fixture:drain-requested\n", started + elapsed);
        for scenario in ["blocked-current-thread", "blocked-request-failure"] {
            let policy = WaitPolicy::for_scenario(scenario);
            for polled_at in [started + STARTUP_LIMIT, started + Duration::from_secs(12)] {
                let deadline = policy
                    .deadline(&output, started, polled_at)
                    .unwrap()
                    .unwrap();
                assert_eq!(deadline - elapsed, OBSERVATION_LIMIT);
                assert!(deadline <= policy.maximum_wait());
                assert!(deadline < launch::EMERGENCY_LIMIT);
            }
        }
    }
}

#[test]
fn synchronization_budget_includes_panic_after_latest_accepted_startup() {
    let started = Instant::now();
    let mut output = Output::default();
    output.record(b"batter-fixture:drain-requested\n", started + STARTUP_LIMIT);
    let panic_at = started + STARTUP_LIMIT + fixture::BLOCKED_OBSERVATION;
    output.record(
        b"thread 'fixture' panicked at deliberate request failure\n",
        panic_at,
    );
    let policy = WaitPolicy::for_scenario("blocked-request-failure");
    assert!(panic_at < started + policy.maximum_wait());
    assert_eq!(
        policy.deadline(&output, started, panic_at).unwrap(),
        Some(policy.maximum_wait())
    );
    assert!(policy.maximum_wait() < launch::EMERGENCY_LIMIT);
    assert!(output.check().unwrap_err().contains("child panicked"));
}

#[test]
fn a_genuinely_late_record_cannot_rescue_expired_startup() {
    let started = Instant::now();
    let late = started + STARTUP_LIMIT + Duration::from_nanos(1);
    let mut output = Output::default();
    output.record(b"batter-fixture:drain-requested\n", late);
    output.record(b"thread 'fixture' panicked at deliberate failure\n", late);
    let error = WaitPolicy::for_scenario("blocked-current-thread")
        .deadline(&output, started, late)
        .unwrap_err();
    assert!(error.contains("after its deadline"), "{error}");
}

#[test]
fn absent_startup_is_pending_before_the_deadline_and_fails_at_it() {
    let started = Instant::now();
    let policy = WaitPolicy::for_scenario("blocked-current-thread");
    let output = Output::default();
    assert_eq!(
        policy
            .deadline(
                &output,
                started,
                started + STARTUP_LIMIT - Duration::from_nanos(1)
            )
            .unwrap(),
        None
    );
    for now in [started + STARTUP_LIMIT, started + launch::EMERGENCY_LIMIT] {
        let error = policy.deadline(&output, started, now).unwrap_err();
        assert!(error.contains("did not emit drain-requested"), "{error}");
    }
}

#[test]
fn ordinary_scenarios_keep_their_original_spawn_deadline() {
    let started = Instant::now();
    let output = Output::default();
    for scenario in [
        "cooperative",
        "non-yielding",
        "stuck-before-runtime",
        "fixture-failure",
        "exit-success",
        "exit-failure",
        "unexpected-cleanup",
        "output-overflow",
    ] {
        assert_eq!(
            WaitPolicy::for_scenario(scenario)
                .deadline(&output, started, started + Duration::from_secs(20))
                .unwrap(),
            Some(EXIT_LIMIT)
        );
    }
}

#[test]
fn event_only_wait_returns_observation_time_without_resetting_it() {
    let started = Instant::now();
    let at = started + Duration::from_secs(2);
    let mut output = Output::default();
    output.record(b"batter-fixture:fixture-entered\n", at);
    assert_eq!(
        WaitPolicy::until_event("fixture-entered")
            .deadline(&output, started, started + Duration::from_secs(20))
            .unwrap(),
        Some(Duration::from_secs(2))
    );
}
