// Attach HTTP-owned controls to the exact shared implementation.
include!("../../../../test-support/process/watchdog.rs");

fn ordinary_command(args: &[&str]) -> ChildRun {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command.args(args).env("BATTER_HTTP_LIFETIME_CASE", "stall");
    let mut child = FixtureChild::spawn_command("ordinary-discovery", command, None).unwrap();
    child.wait(Duration::from_secs(2)).unwrap()
}

#[test]
fn zero_selected_tests_cannot_prove_http_success() {
    let run = ordinary_command(&["--exact", "no-such-http-fixture"]);
    assert!(run.status.success(), "{run:?}");
    assert!(run.output.text().contains("running 0 tests"));
    assert!(
        super::success(&run, "idle_keep_alive_drain")
            .unwrap_err()
            .contains("missing event")
    );
}

#[test]
fn ambient_scenario_does_not_authorize_ordinary_child_discovery() {
    let run = ordinary_command(&["--exact", "child_fixture"]);
    assert!(run.status.success(), "{run:?}");
    assert!(!run.kill_requested());
    assert!(run.output.text().contains("running 1 test"));
    assert!(!run.output.text().contains("runtime-stall"));
    assert!(super::success(&run, "stall").is_err());
}
