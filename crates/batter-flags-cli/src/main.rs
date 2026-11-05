//! `batter-flags`, the operator command for the flags control plane.
//!
//! Scaffold: the operator commands arrive with the control-plane delivery
//! tasks. The root already fixes its panic policy: as a root that consumes
//! Batter, the first statement of `main` installs Batter's payload-free
//! `redacting_panic_hook`, before any argument parsing, configuration or
//! runtime construction exists.
#![forbid(unsafe_code)]
#![deny(
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    clippy::unwrap_used
)]

use std::{
    io::{self, Write},
    process::ExitCode,
};

fn main() -> ExitCode {
    std::panic::set_hook(batter_core::redacting_panic_hook());
    // Ignored: nothing further can be reported when standard error is closed.
    let _ =
        io::stderr().write_all(b"batter-flags: this scaffold implements no operator command yet\n");
    ExitCode::from(2)
}
