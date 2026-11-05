//! The redacting panic hook in fresh child processes, launched through the
//! shared fixture protocol because a panic hook is process-global. Every child
//! except `entrypoints` panics with a secret-shaped payload and records each
//! panic's location on standard output first; the parent reads standard output
//! and standard error separately. With the redacting hook installed, standard
//! error must be exactly the redacted lines, or empty at the descriptor limit;
//! the default-hook controls must instead print the payloads.
#![allow(clippy::unwrap_used, reason = "a panic fails the test")]
#![allow(clippy::missing_assert_message, reason = "test names locate failures")]

#[allow(dead_code, reason = "shared support; this crate uses part of it")]
#[path = "../../../test-support/process/capture.rs"]
mod capture;
#[path = "panic_hook/child.rs"]
mod child;
#[allow(dead_code, reason = "shared support; this crate uses part of it")]
#[path = "../../../test-support/process/evidence.rs"]
mod evidence;
#[path = "panic_hook/fixture.rs"]
mod fixture;
#[path = "../../../test-support/process/launch.rs"]
mod launch;

use fixture::{Exit, Launch};

#[test]
fn child_fixture() {
    if let Some(scenario) = launch::scenario() {
        child::run(&scenario);
    }
}

/// Negative control: Rust's default hook prints all three string payloads and
/// every recorded location, so the oracle below would see a disclosure.
#[test]
fn default_hook_control_prints_the_payloads() {
    let finished = fixture::run("default-hook", Launch::default());
    finished.expect(
        Exit::Success,
        &[
            "unwound:literal",
            "unwound:message",
            "unwound:unwrap-debug",
            "unwound:opaque",
            "payloads-intact",
        ],
    );
    let stderr = finished.stderr().text();
    if stderr.matches(child::SECRET_CORE).count() != 3 {
        finished.fail("the default hook did not print the three string payloads");
    }
    let sites = finished.sites();
    if sites.len() != 4 || !sites.iter().all(|site| stderr.contains(site.as_str())) {
        finished.fail("the default hook did not report every recorded location");
    }
}

#[test]
fn caught_panics_report_each_location_and_no_payload() {
    let launch = Launch {
        backtrace: Some("1"),
        ..Launch::default()
    };
    let finished = fixture::run("caught", launch);
    finished
        .expect(
            Exit::Success,
            &[
                "hook-installed",
                "unwound:literal",
                "unwound:message",
                "unwound:unwrap-debug",
                "unwound:opaque",
                "payloads-intact",
            ],
        )
        .expect_redacted(4);
    // Repository panic detectors still classify the redacted line as a panic.
    assert!(finished.stderr().check().is_err());
}

/// The libtest test-thread boundary: libtest's failure status equals std's
/// status for a panic escaping `main`. A real `fn main` is not executed here.
#[test]
fn an_uncaught_panic_ends_the_process_with_the_same_status() {
    let control = fixture::run("uncaught-default", Launch::default());
    control.expect(Exit::UncaughtPanic, &["unwound:uncaught"]);
    let stderr = control.stderr().text();
    let sites = control.sites();
    if !stderr.contains(child::SECRET) || sites.len() != 1 || !stderr.contains(&sites[0]) {
        control.fail("the default hook did not print the payload and location");
    }
    let finished = fixture::run("uncaught", Launch::default());
    finished
        .expect(Exit::UncaughtPanic, &["hook-installed", "unwound:uncaught"])
        .expect_redacted(1);
    assert_eq!(finished.status().code(), control.status().code());
}

#[test]
fn tokio_task_panics_unwind_into_join_errors_without_their_payload() {
    for (scenario, thread) in [
        ("tokio-current-thread", "task-on-caller-thread"),
        ("tokio-multi-thread", "task-on-worker-thread"),
    ] {
        fixture::run(scenario, Launch::default())
            .expect(
                Exit::Success,
                &["hook-installed", "unwound:task", thread, "payloads-intact"],
            )
            .expect_redacted(1);
    }
}

#[test]
fn a_spawned_thread_panic_reaches_its_join_with_the_payload_intact() {
    fixture::run("std-thread", Launch::default())
        .expect(
            Exit::Success,
            &["hook-installed", "unwound:thread", "payloads-intact"],
        )
        .expect_redacted(1);
}

/// The report does not wait for the `Stderr` lock that another thread holds
/// from before the panic until after the report; a hook that took the lock
/// would block until the parent's guard killed the child.
#[test]
fn the_report_does_not_wait_for_a_held_stderr_lock() {
    fixture::run("stderr-lock-held", Launch::default())
        .expect(
            Exit::Success,
            &[
                "hook-installed",
                "unwound:lock-held",
                "reported-while-locked",
                "payloads-intact",
            ],
        )
        .expect_redacted(1);
}

/// With no descriptor left to duplicate standard error, the hook writes
/// nothing instead of taking the `Stderr` lock; the panic still unwinds with
/// its payload.
#[test]
fn at_the_descriptor_limit_the_hook_writes_nothing() {
    let launch = Launch {
        descriptor_limit: Some(32),
        ..Launch::default()
    };
    fixture::run("descriptor-limit", launch)
        .expect(
            Exit::Success,
            &[
                "hook-installed",
                "descriptors-exhausted",
                "unwound:descriptor-limit",
                "payloads-intact",
            ],
        )
        .expect_silent(1);
}

/// Constructing the hook installs nothing, and Batter entrypoints that own
/// process-wide signal handlers or catch panics keep the application's hook
/// installed and unwrapped, passing each panic they catch to it once. The
/// in-child controls show that the identity check sees a wrapper and the
/// count sees a hook swapped out around a panic.
#[test]
fn batter_entrypoints_keep_the_application_installed_hook() {
    let finished = fixture::run("entrypoints", Launch::default());
    finished
        .expect(
            Exit::Success,
            &[
                "hook-kept:installed",
                "hook-kept:constructor",
                "hook-kept:command",
                "hook-kept:startup",
                "hook-kept:run-blocking",
                "control:swap-counted",
                "control:wrapper-seen",
            ],
        )
        .expect_redacted(0);
}
