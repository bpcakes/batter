use std::process::Command;

#[test]
fn missing_configuration_exits_with_a_redacted_process_error() {
    let output = Command::new(env!("CARGO_BIN_EXE_postgres_lifecycle"))
        .env_remove("DATABASE_URL")
        .output()
        .expect("example process must start");

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert_eq!(output.stderr, b"Error: process failed\n");
}
