//! HTTP observation events, correlation and probe rendering.
#![allow(clippy::unwrap_used, reason = "a panic fails the test")]
#![allow(clippy::missing_assert_message, reason = "test names locate failures")]
#[path = "../../../test-support/capture.rs"]
mod capture;
#[path = "observation/composition.rs"]
mod composition;
#[path = "observation/composition_edges.rs"]
mod composition_edges;
#[path = "observation/correlation.rs"]
mod correlation;
#[path = "observation/event_fields.rs"]
mod event_fields;
#[path = "observation/interruption.rs"]
mod interruption;
#[path = "observation/lifetime.rs"]
mod lifetime;
#[path = "observation/placement.rs"]
mod placement;
#[path = "observation/rendering.rs"]
mod rendering;
#[path = "observation/severity.rs"]
mod severity;
#[path = "observation/stacked_interruption.rs"]
mod stacked_interruption;
#[path = "observation/support.rs"]
mod support;
