//! `runflag`, the operator command for the flag control plane.
//!
//! Scaffold: the operator commands arrive with the control-plane delivery
//! tasks. The root already fixes its panic policy. The first statement of
//! `main` installs `payload_free_hook`, which reports where a panic happened
//! and never what it carried, before any argument parsing, configuration or
//! runtime construction exists.

use std::{
    fs::File,
    io::{self, Write},
    os::fd::AsFd,
    panic::PanicHookInfo,
    process::ExitCode,
};

fn main() -> ExitCode {
    std::panic::set_hook(Box::new(payload_free_hook));
    // Ignored: nothing further can be reported when standard error is closed.
    let _ = io::stderr().write_all(b"runflag: this scaffold implements no operator command yet\n");
    ExitCode::from(2)
}

/// Report the panic location on standard error and never the payload.
///
/// The line goes through a duplicate of the standard error descriptor rather
/// than the `Stderr` lock, which another thread can hold indefinitely; when no
/// descriptor can be duplicated the hook writes nothing. Write errors are
/// ignored and no thread state is read, so the hook itself does not panic. It
/// calls neither the previous nor the default hook and returns normally, so
/// unwinding and the exit status of an uncaught panic are unchanged.
fn payload_free_hook(info: &PanicHookInfo<'_>) {
    let Ok(descriptor) = io::stderr().as_fd().try_clone_to_owned() else {
        return;
    };
    let mut output = File::from(descriptor);
    let _ = match info.location() {
        Some(location) => write!(
            output,
            "\nrunflag panicked at {}:{}:{}; panic payload redacted\n",
            location.file(),
            location.line(),
            location.column()
        ),
        None => {
            output.write_all(b"\nrunflag panicked at an unknown location; panic payload redacted\n")
        }
    };
}
