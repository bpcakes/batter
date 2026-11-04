//! Browser origin, cookie, mutation-signal and private-response mechanics.
#![allow(clippy::unwrap_used, reason = "a panic fails the test")]
#![allow(clippy::missing_assert_message, reason = "test names locate failures")]
#[path = "../../../test-support/capture.rs"]
mod capture;
#[path = "browser/cookie_input.rs"]
mod cookie_input;
#[path = "browser/cookie_output.rs"]
mod cookie_output;
#[path = "browser/mutation.rs"]
mod mutation;
#[path = "browser/origin.rs"]
mod origin;
#[path = "browser/private_response.rs"]
mod private_response;
