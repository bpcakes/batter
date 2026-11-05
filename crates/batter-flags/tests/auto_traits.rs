//! The types a gate closure or a registered component holds are shareable:
//! the protected surfaces build `Fn() -> _ + Send + Sync + 'static` closures
//! over `Flags`, and a root hands errors across tasks.
#![allow(clippy::missing_assert_message, reason = "test names locate failures")]

use batter_flags::{
    Flags, GateError, StartupReport, ValueSource,
    declared::{DeclaredError, Resolved, ValueProblem},
    test::{FixtureError, TestFlags, TestFlagsBuilder},
};

fn shareable<T: Send + Sync + 'static>() {}

fn cloneable<T: Clone>() {}

#[test]
fn the_flags_and_every_error_are_send_sync_and_static() {
    shareable::<Flags>();
    shareable::<Resolved>();
    shareable::<StartupReport>();
    shareable::<ValueSource>();
    shareable::<DeclaredError>();
    shareable::<ValueProblem>();
    shareable::<GateError>();
    shareable::<FixtureError>();
    shareable::<TestFlags>();
    shareable::<TestFlagsBuilder>();
}

#[test]
fn the_flags_clone_cheaply_and_resolved_does_not_clone() {
    cloneable::<Flags>();
    cloneable::<GateError>();
    cloneable::<StartupReport>();
}
