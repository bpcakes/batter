//! Public contracts for library-owned periodic maintenance components.
#![allow(clippy::unwrap_used, reason = "a panic fails the test")]
#![allow(clippy::missing_assert_message, reason = "test names locate failures")]

#[path = "periodic/outcomes.rs"]
mod outcomes;
#[path = "periodic/registration.rs"]
mod registration;
#[path = "periodic/schedule.rs"]
mod schedule;
#[path = "periodic/settlement.rs"]
mod settlement;
#[path = "periodic/shutdown.rs"]
mod shutdown;
#[path = "periodic/skips.rs"]
mod skips;
#[path = "periodic/startup.rs"]
mod startup;
#[path = "periodic/support.rs"]
mod support;
