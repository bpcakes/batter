//! Guarded route groups with their own request and browser policy.
#[path = "route_groups/admitted.rs"]
mod admitted;
#[path = "route_groups/admitted_validation.rs"]
mod admitted_validation;
#[path = "route_groups/browser.rs"]
mod browser;
#[path = "../../../test-support/capture.rs"]
mod capture;
#[path = "route_groups/consumer.rs"]
mod consumer;
#[path = "route_groups/guarantees.rs"]
mod guarantees;
#[path = "route_groups/support.rs"]
mod support;
#[path = "route_groups/validation.rs"]
mod validation;
