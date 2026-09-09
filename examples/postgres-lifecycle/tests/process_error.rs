use std::{path::Path, process::Command};

#[test]
fn missing_configuration_exits_with_a_redacted_process_error() {
    // Keep the exact stderr/stdout assertions under an external child watchdog.
    let status = Command::new("python3")
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/diagnostics.py"))
        .arg("missing-configuration")
        .arg(env!("CARGO_BIN_EXE_postgres_lifecycle"))
        .status()
        .expect("external diagnostic watchdog must execute");
    assert!(
        status.success(),
        "missing-configuration exit control failed"
    );
}
