use std::{path::Path, process::Command};

fn run_control(name: &str) {
    // The Python parent owns a wall-clock watchdog independently of the example
    // runtime, with bounded capture and explicit kill/reap on incomplete exit.
    let status = Command::new("python3")
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/diagnostics.py"))
        .arg(name)
        .arg(env!("CARGO_BIN_EXE_postgres_lifecycle"))
        .status()
        .expect("external diagnostic watchdog must execute");
    assert!(status.success(), "diagnostic control failed: {name}");
}

#[test]
fn configuration_failures_exit_without_printing_credentials() {
    run_control("configuration");
}

#[test]
fn diagnostic_watchdog_kills_and_reaps_a_hung_child() {
    run_control("watchdog");
}
