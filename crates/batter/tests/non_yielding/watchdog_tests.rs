use super::*;
use std::process::Stdio;

fn unlaunched_child() -> FixtureChild {
    let (input, parent_pipe) = io::pipe().unwrap();
    let mut command = Command::new(std::env::current_exe().unwrap());
    command.args(launch::CHILD_ARGS).stdin(input);
    FixtureChild::spawn_command("unlaunched", command, Some(parent_pipe)).unwrap()
}

#[test]
fn startup_polled_after_the_child_panic_reaches_final_validation() {
    let mut child = FixtureChild::spawn("blocked-request-failure").unwrap();
    let limit = WaitPolicy::for_scenario(&child.scenario).maximum_wait();
    // Synchronize on actual capture, not an assumed delay. Both drain and the
    // deliberate panic must be buffered before the first startup-policy poll.
    while !child
        .capture
        .snapshot()
        .text()
        .contains("deliberate request failure")
    {
        assert!(child.started.elapsed() < limit, "panic was not captured");
        std::thread::sleep(POLL);
    }
    let run = child.wait_for_scenario().unwrap();
    let drain = run.output.observed("drain-requested").unwrap();
    assert_eq!(
        child.started + run.deadline_after_start,
        drain + OBSERVATION_LIMIT
    );
    assert!(run.kill_requested() && killed_status(run.status), "{run:?}");
    let error = run
        .validate(ExpectedExit::WatchdogKill, &["drain-requested"], &[])
        .unwrap_err();
    assert!(error.contains("child panicked") && error.contains("deliberate request failure"));
}

#[test]
fn blocked_runtime_wait_uses_the_captured_drain_deadline() {
    for scenario in ["blocked-current-thread", "blocked-request-failure"] {
        let mut child = FixtureChild::spawn(scenario).unwrap();
        let started = child.started;
        let run = child.wait_for_scenario().unwrap();
        let drain = run.output.observed("drain-requested").unwrap();
        // Inspect the deadline actually passed to wait(), independently of the
        // policy calculation. Fast startup must not hide a fixed-deadline reset.
        assert_eq!(
            started + run.deadline_after_start,
            drain + OBSERVATION_LIMIT,
            "{run:?}"
        );
        assert!(killed_status(run.status), "{run:?}");
        assert!(
            run.kill_requested_at
                .is_some_and(|at| at >= run.deadline_after_start),
            "{run:?}"
        );
        // A correct recorded deadline must not hide waiting for a later bound.
        // This allowance tolerates scheduling delay; it is not an OS guarantee.
        assert!(
            run.kill_requested_at.unwrap() < run.deadline_after_start + OBSERVATION_LIMIT,
            "watchdog kill was late: {run:?}"
        );
    }
}

#[test]
fn blocked_runtime_startup_cannot_wait_for_the_emergency_exit() {
    let mut child = unlaunched_child();
    child.scenario = "blocked-current-thread".into();
    let started = child.started;
    let failure = std::thread::spawn(move || child.wait_for_scenario()).join();
    let panic = failure.expect_err("missing startup evidence must fail");
    let message = panic.downcast_ref::<String>().unwrap();
    assert!(
        message.contains("child did not emit drain-requested"),
        "{message}"
    );
    let elapsed = started.elapsed();
    assert!(
        elapsed >= EXIT_LIMIT && elapsed < launch::EMERGENCY_LIMIT,
        "{elapsed:?}"
    );
}

#[test]
fn parent_unwind_kills_reaps_and_joins_capture_reader() {
    let mut child = FixtureChild::spawn("stuck-before-runtime").unwrap();
    let pid = child.child.id();
    let started = child.started;
    child.wait_for_event("fixture-entered");
    let failure = std::thread::spawn(move || {
        let _child = child;
        panic!("deliberate parent assertion failure");
    })
    .join();
    assert!(failure.is_err());
    // Include startup in the five-second bound: waiting for the ten-second
    // child emergency exit must fail even if unwinding starts late.
    let elapsed = started.elapsed();
    assert!(
        elapsed < EXIT_LIMIT,
        "unwind cleanup took {elapsed:?}; expected completion before {EXIT_LIMIT:?}"
    );
    // Linux /proc retains zombies: absence proves both termination and reap.
    #[cfg(target_os = "linux")]
    assert!(!std::path::PathBuf::from(format!("/proc/{pid}")).exists());
    #[cfg(not(target_os = "linux"))]
    let _ = pid;
}

#[test]
fn parent_pipe_closure_ends_blocked_runtime_without_cleanup() {
    let mut child = FixtureChild::spawn("non-yielding").unwrap();
    child.wait_for_event("runtime-drop-started");
    drop(child.parent_pipe.take());
    child
        .wait(EXIT_LIMIT)
        .unwrap()
        .validate(
            ExpectedExit::Code(launch::PARENT_GONE),
            &["report-unjoined-cleanup-skipped", "runtime-drop-started"],
            &["task-dropped", "cleanup-invoked", "runtime-dropped"],
        )
        .unwrap();
}

#[test]
fn emergency_deadline_covers_blocked_work_and_incomplete_launch() {
    // Start together so the two independent ten-second bounds overlap.
    let mut blocked = FixtureChild::spawn("stuck-before-runtime").unwrap();
    let mut unlaunched = unlaunched_child();
    blocked.wait_for_event("fixture-entered");
    for child in [&mut blocked, &mut unlaunched] {
        let run = child.wait(launch::EMERGENCY_LIMIT + EXIT_LIMIT).unwrap();
        run.validate(ExpectedExit::Code(launch::EMERGENCY_DEADLINE), &[], &[])
            .unwrap();
        assert!(run.elapsed >= launch::EMERGENCY_LIMIT, "{run:?}");
        assert!(child.parent_pipe.is_some());
    }
    assert!(
        !unlaunched
            .capture
            .snapshot()
            .text()
            .contains("fixture-entered")
    );
}

#[test]
fn launch_rejects_a_record_for_another_process() {
    let mut child = unlaunched_child();
    writeln!(
        child.parent_pipe.as_mut().unwrap(),
        "batter-fixture-v1 0 stuck-before-runtime"
    )
    .unwrap();
    child
        .wait(EXIT_LIMIT)
        .unwrap()
        .validate(
            ExpectedExit::Code(launch::INVALID_LAUNCH),
            &[],
            &["fixture-entered"],
        )
        .unwrap();
}

#[test]
fn exported_scenario_cannot_activate_an_ordinary_test_entry() {
    // Historical regression: an earlier fixture trusted this ambient variable.
    // Keep the hostile input even though the current launch code never reads it.
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", "child_fixture", "--test-threads=1"])
        .env("BATTER_NON_YIELDING_SCENARIO", "stuck-before-runtime")
        .stdin(Stdio::null());
    FixtureChild::spawn_command("ambient-scenario", command, None)
        .unwrap()
        .wait(EXIT_LIMIT)
        .unwrap()
        .validate(ExpectedExit::Success, &[], &["fixture-entered"])
        .unwrap();
}

#[test]
fn natural_exits_remain_distinct_after_a_later_kill_request() {
    let mut success = FixtureChild::spawn("exit-success").unwrap();
    let mut failure = FixtureChild::spawn("exit-failure").unwrap();
    for (child, code) in [(&mut success, 0), (&mut failure, 7)] {
        let run = child.wait(EXIT_LIMIT).unwrap();
        run.validate(ExpectedExit::Code(code), &[], &[]).unwrap();
        assert!(run.validate(ExpectedExit::WatchdogKill, &[], &[]).is_err());
        if code != 0 {
            assert!(run.validate(ExpectedExit::Success, &[], &[]).is_err());
        }
    }
    // Force the exit-before-kill interleaving without relying on scheduling a
    // process inside the 10 ms polling window. Kill must preserve these reaped
    // OS statuses even after the normal watchdog deadline has elapsed.
    std::thread::sleep(EXIT_LIMIT.saturating_sub(failure.started.elapsed()));
    for (child, code) in [(&mut success, 0), (&mut failure, 7)] {
        assert_eq!(child.kill_and_reap().unwrap().code(), Some(code));
        let run = child.wait(EXIT_LIMIT).unwrap();
        assert!(run.kill_requested() && run.elapsed >= EXIT_LIMIT, "{run:?}");
        let error = run
            .validate(ExpectedExit::WatchdogKill, &[], &[])
            .unwrap_err();
        assert!(error.contains("unexpected"), "{error}");
    }
}

#[test]
fn watchdog_rejects_a_kill_before_its_deadline() {
    let mut child = FixtureChild::spawn("stuck-before-runtime").unwrap();
    child.wait_for_event("fixture-entered");
    child.kill_and_reap().unwrap();
    let run = child.wait(EXIT_LIMIT).unwrap();
    assert!(
        killed_status(run.status) && run.elapsed < EXIT_LIMIT,
        "{run:?}"
    );
    assert!(run.validate(ExpectedExit::WatchdogKill, &[], &[]).is_err());
}

#[test]
fn watchdog_rejects_a_kill_before_an_extended_observation_deadline() {
    let mut child = FixtureChild::spawn("stuck-before-runtime").unwrap();
    child.wait_for_event("fixture-entered");
    std::thread::sleep(EXIT_LIMIT.saturating_sub(child.started.elapsed()));
    child.kill_and_reap().unwrap();
    let deadline = EXIT_LIMIT + OBSERVATION_LIMIT;
    let run = child.wait(deadline).unwrap();
    assert!(killed_status(run.status), "{run:?}");
    assert!(
        run.elapsed >= EXIT_LIMIT && run.elapsed < deadline,
        "{run:?}"
    );
    assert!(run.validate(ExpectedExit::WatchdogKill, &[], &[]).is_err());
}

#[test]
fn delayed_capture_cannot_make_an_early_kill_look_timely() {
    let mut run = ChildRun {
        status: ExitStatus::from_raw(9),
        kill_requested_at: Some(Duration::from_secs(2)),
        scenario: "early-kill".into(),
        elapsed: Duration::from_secs(20),
        deadline_after_start: EXIT_LIMIT,
        output: Output::default(),
    };
    assert!(run.validate(ExpectedExit::WatchdogKill, &[], &[]).is_err());
    run.kill_requested_at = Some(EXIT_LIMIT);
    run.validate(ExpectedExit::WatchdogKill, &[], &[]).unwrap();
    run.kill_requested_at = None;
    assert!(run.validate(ExpectedExit::WatchdogKill, &[], &[]).is_err());
}

#[test]
fn timely_kill_requests_do_not_turn_other_exit_statuses_into_sigkill() {
    // Unix wait statuses: ordinary exits plus SIGTERM, SIGALRM and SIGABRT.
    // Real natural-exit/kill races are covered above; this isolates classification.
    for raw in [0, 7 << 8, 15, 14, 6] {
        let run = ChildRun {
            status: ExitStatus::from_raw(raw),
            kill_requested_at: Some(EXIT_LIMIT),
            scenario: "different-termination".into(),
            elapsed: EXIT_LIMIT,
            deadline_after_start: EXIT_LIMIT,
            output: Output::default(),
        };
        assert!(run.validate(ExpectedExit::WatchdogKill, &[], &[]).is_err());
    }
}

#[test]
fn capture_overflow_also_invalidates_a_timely_watchdog_kill() {
    let mut output = Output::default();
    output.record(b"batter-fixture:evidence-ready\n", Instant::now());
    output.record(&vec![b'x'; 128 * 1024], Instant::now());
    let run = ChildRun {
        status: ExitStatus::from_raw(9),
        kill_requested_at: Some(EXIT_LIMIT),
        scenario: "overflow-before-kill".into(),
        elapsed: EXIT_LIMIT,
        deadline_after_start: EXIT_LIMIT,
        output,
    };
    let error = run
        .validate(ExpectedExit::WatchdogKill, &["evidence-ready"], &[])
        .unwrap_err();
    assert!(error.contains("capture limit"), "{error}");
}

#[test]
fn forbidden_events_reject_otherwise_successful_evidence() {
    let run = ChildRun::run("unexpected-cleanup");
    run.validate(ExpectedExit::Success, &["evidence-ready"], &[])
        .unwrap();
    let error = run
        .validate(
            ExpectedExit::Success,
            &["evidence-ready"],
            &["cleanup-invoked"],
        )
        .unwrap_err();
    assert!(
        error.contains("unexpected event cleanup-invoked"),
        "{error}"
    );
}

#[test]
fn capture_overflow_cannot_hide_late_forbidden_evidence() {
    let run = ChildRun::run("output-overflow");
    assert!(run.status.success() && !run.kill_requested(), "{run:?}");
    let error = run
        .validate(
            ExpectedExit::Success,
            &["evidence-ready"],
            &["cleanup-invoked"],
        )
        .unwrap_err();
    assert!(error.contains("output exceeded capture limit"), "{error}");
}

#[test]
fn event_overflow_keeps_draining_and_preserves_its_cause() {
    let run = ChildRun::run("event-overflow");
    // The fixture fills the pipe after exceeding the event bound. Only a reader
    // that keeps draining allows an ordinary exit rather than a watchdog kill.
    assert!(run.status.success() && !run.kill_requested(), "{run:?}");
    let error = run
        .validate(
            ExpectedExit::Success,
            &["evidence-ready"],
            &["cleanup-invoked"],
        )
        .unwrap_err();
    assert!(error.contains("distinct events"), "{error}");
}

#[test]
fn event_wait_accepts_final_output_from_an_exited_child() {
    let mut child = FixtureChild::spawn("exit-success").unwrap();
    child.child.wait().unwrap();
    child.wait_for_event("fixture-entered");
    child
        .wait(EXIT_LIMIT)
        .unwrap()
        .validate(ExpectedExit::Success, &["fixture-entered"], &[])
        .unwrap();
}

#[test]
fn event_wait_rejects_an_exited_child_without_the_event() {
    reject_missing_final_event(false);
}

#[test]
fn exited_child_rejection_precedes_an_expired_startup_deadline() {
    reject_missing_final_event(true);
}

#[test]
fn final_capture_failures_are_not_masked_by_missing_startup() {
    for (scenario, expected) in [
        ("fixture-failure", "child panicked"),
        ("event-overflow", "exceeded capture limit"),
    ] {
        let mut child = FixtureChild::spawn(scenario).unwrap();
        child.child.wait().unwrap();
        let failure = std::thread::spawn(move || child.wait_for_event("absent-event")).join();
        let panic = failure.expect_err("final capture must fail");
        let message = panic.downcast_ref::<String>().unwrap();
        assert!(message.contains(expected), "{message}");
        assert!(
            !message.contains("child exited before startup evidence"),
            "{message}"
        );
    }
}

fn reject_missing_final_event(expired: bool) {
    let mut child = FixtureChild::spawn("exit-success").unwrap();
    child.child.wait().unwrap();
    let failure = std::thread::spawn(move || {
        // Select the policy clock after reaping; process startup latency cannot
        // decide which branch this control exercises.
        child.started = Instant::now();
        if expired {
            child.started -= super::super::timing::STARTUP_LIMIT;
        }
        child.wait_for_event("absent-event");
    })
    .join();
    let panic = failure.expect_err("an exited child cannot supply missing evidence");
    let message = panic.downcast_ref::<String>().unwrap();
    assert!(
        message.contains("child exited before startup evidence"),
        "{message}"
    );
}

#[test]
fn spawn_failure_releases_the_capture_reader() {
    let command = Command::new(std::env::current_exe().unwrap().join("not-a-directory"));
    assert!(FixtureChild::spawn_command("spawn-failure", command, None).is_err());
}

#[cfg(target_os = "linux")]
fn parent_death_probe() -> Command {
    let mut command = Command::new("python3");
    command
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/non_yielding/parent_death.py"
        ))
        .env("PYTHONOPTIMIZE", "1")
        .stdin(Stdio::null());
    command
}

#[cfg(target_os = "linux")]
#[test]
fn abrupt_parent_death_stops_and_reaps_the_orphaned_fixture() {
    let mut command = parent_death_probe();
    command.arg(std::env::current_exe().unwrap());
    FixtureChild::spawn_command("parent-death-probe", command, None)
        .expect("Linux parent-death regression requires Python 3")
        .wait(launch::EMERGENCY_LIMIT + EXIT_LIMIT)
        .unwrap()
        .validate(ExpectedExit::Success, &["parent-death-reaped"], &[])
        .unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn optimized_parent_death_probe_rejects_invalid_child_evidence() {
    for (termination, diagnostic) in [
        ("sys.exit(7)", "unexpected orphan status"),
        (
            "print('batter-fixture:cleanup-invoked', flush=True)\nsys.exit(74)",
            "forbidden orphan event",
        ),
    ] {
        // Supply the real launch/EOF protocol, but deliberately violate the
        // termination contract after the probe kills the owner. Keep a separate
        // bound even if a regression prevents the probe from closing stdin.
        let script = format!(
            "import signal, sys\nsignal.alarm(6)\nsys.stdin.readline()\n\
             print('batter-fixture:report-unjoined-cleanup-skipped', flush=True)\n\
             print('batter-fixture:runtime-drop-started', flush=True)\n\
             sys.stdin.read()\n{termination}\n"
        );
        let mut command = parent_death_probe();
        command.args(["python3", "-c", &script]);
        let run = FixtureChild::spawn_command("invalid-parent-death-evidence", command, None)
            .unwrap()
            .wait(launch::EMERGENCY_LIMIT + EXIT_LIMIT)
            .unwrap();
        run.validate(ExpectedExit::Code(1), &[], &["parent-death-reaped"])
            .unwrap();
        assert!(run.output.text().contains(diagnostic), "{run:?}");
    }
}
