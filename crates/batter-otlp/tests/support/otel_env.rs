//! Run adapter tests in a child when the invoking shell has OTEL settings.
//!
//! The public preparation path correctly rejects those settings. A child lets
//! the tests exercise that real path without changing the environment of other
//! tests running in the same process.

use std::{ffi::OsString, process::Command};

fn ambient_names() -> Vec<OsString> {
    std::env::vars_os()
        .filter_map(|(name, _)| {
            name.as_encoded_bytes()
                .starts_with(b"OTEL_")
                .then_some(name)
        })
        .collect()
}

/// Select one exact test in a child with every inherited OTEL setting removed.
pub fn clean_child(test: &str) -> Command {
    let mut command = Command::new(std::env::current_exe().expect("test executable path"));
    command.args(["--exact", test, "--nocapture"]);
    for name in ambient_names() {
        command.env_remove(name);
    }
    command
}

/// Return true in the ambient parent after its clean child ran the full test.
pub fn rerun_if_ambient() -> bool {
    if ambient_names().is_empty() {
        return false;
    }
    let name = std::thread::current()
        .name()
        .expect("test thread has a name")
        .to_owned();
    let status = clean_child(&name).status().expect("isolated test starts");
    assert!(status.success(), "isolated {name} failed: {status}");
    true
}
