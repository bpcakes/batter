//! Guarded route groups with their own request and browser policy.
#![allow(clippy::unwrap_used, reason = "a panic fails the test")]
#![allow(clippy::missing_assert_message, reason = "test names locate failures")]
#[path = "route_groups/admitted.rs"]
mod admitted;
#[path = "route_groups/admitted_dispatch.rs"]
mod admitted_dispatch;
#[path = "route_groups/admitted_validation.rs"]
mod admitted_validation;
#[path = "route_groups/browser.rs"]
mod browser;
#[path = "../../../test-support/capture.rs"]
mod capture;
#[path = "route_groups/consumer.rs"]
mod consumer;
#[path = "route_groups/credentials.rs"]
mod credentials;
#[path = "route_groups/guarantees.rs"]
mod guarantees;
#[path = "route_groups/reused_request_state.rs"]
mod reused_request_state;
#[path = "route_groups/reused_routing.rs"]
mod reused_routing;
#[path = "route_groups/reused_routing_accepted.rs"]
mod reused_routing_accepted;
#[path = "route_groups/support.rs"]
mod support;
#[path = "route_groups/validation.rs"]
mod validation;
