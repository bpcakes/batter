#[allow(dead_code)]
#[path = "../../../../test-support/process/capture.rs"]
mod capture;
#[allow(dead_code)]
#[path = "../../../../test-support/process/evidence.rs"]
mod evidence;
#[allow(dead_code)]
#[path = "../../../../test-support/process/launch.rs"]
pub(super) mod launch;
#[path = "../../../../test-support/temp_dir.rs"]
mod temp_dir;
#[allow(dead_code)]
#[path = "../../../../test-support/process/timing.rs"]
mod timing;
#[allow(dead_code)]
mod watchdog {
    include!("../../../../test-support/process/watchdog.rs");

    pub fn check(env: &[(&str, std::ffi::OsString)], expected: &str) {
        let (input, parent_pipe) = io::pipe().unwrap();
        let mut command = Command::new(std::env::current_exe().unwrap());
        command.args(launch::CHILD_ARGS).stdin(input).env_clear();
        command.envs(env.iter().map(|(key, value)| (key, value)));
        let mut child =
            FixtureChild::spawn_command("invalid-settings", command, Some(parent_pipe)).unwrap();
        writeln!(
            child.parent_pipe.as_mut().unwrap(),
            "batter-fixture-v1 {} invalid-settings",
            child.child.id()
        )
        .unwrap();
        let run = child.wait(Duration::from_secs(5)).unwrap();
        run.validate(ExpectedExit::Code(42), &[], &[]).unwrap();
        let text = run.output.text();
        assert!(text.contains("Error: configuration failed:"), "{run:?}");
        assert!(text.contains(expected), "{run:?}");
        assert!(!text.contains("secret-marker"), "{run:?}");
        assert!(!text.contains("HTTP listener bound"), "{run:?}");
    }
}

#[test]
fn actual_entrypoint_rejects_settings_without_exposing_values_or_opening_listener() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};
    for (name, value, diagnostic) in [
        (
            "BATTER_BIND",
            "secret-marker",
            "BATTER_BIND: invalid address",
        ),
        (
            "BATTER_REQUEST_TIMEOUT_MS",
            "secret-marker",
            "BATTER_REQUEST_TIMEOUT_MS:",
        ),
        ("BATTER_UNKNOWN", "secret-marker", "source: unknown key"),
        (
            "RUST_LOG",
            "secret-marker=invalid-level",
            "RUST_LOG: invalid configuration",
        ),
        (
            "BATTER_ENV_FILE",
            "/missing/secret-marker",
            "file: cannot open",
        ),
    ] {
        watchdog::check(&[(name, value.into())], diagnostic);
    }
    watchdog::check(
        &[(
            "BATTER_BIND",
            OsString::from_vec(b"secret-marker\xff".to_vec()),
        )],
        "BATTER_BIND: non-Unicode value",
    );
    watchdog::check(
        &[(
            "RUST_LOG",
            OsString::from_vec(b"secret-marker\xff".to_vec()),
        )],
        "RUST_LOG: non-Unicode value",
    );
    let directory = temp_dir::TempDir::new().unwrap();
    let root = directory.path();
    let path = root.join("secret-marker.env");
    for (text, diagnostic) in [
        ("secret-marker=secret-marker", "file: invalid key syntax"),
        ("SECRET_MARKER=secret-marker", "source: unknown key"),
        ("BATTER_BIND='secret-marker", "file: unmatched quotes"),
        (
            "BATTER_BIND=secret-marker\nBATTER_BIND=localhost",
            "file: duplicate key",
        ),
    ] {
        std::fs::write(&path, text).unwrap();
        watchdog::check(
            &[("BATTER_ENV_FILE", path.clone().into_os_string())],
            diagnostic,
        );
    }
    directory.close().unwrap();
}
