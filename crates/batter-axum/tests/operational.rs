//! Operational HTTP serving: admission, readiness probes, correlation and rendering.
#![allow(clippy::unwrap_used, reason = "a panic fails the test")]
#![allow(clippy::missing_assert_message, reason = "test names locate failures")]
#[path = "../../../test-support/capture.rs"]
mod capture;

mod operational {
    mod admitted_request;
    mod connect_info;
    mod correlation;
    mod group_gate;
    mod group_gate_order;
    mod lifecycle_readiness;
    mod probe_policy;
    mod readiness;
    mod rendered_fallback;
    mod rendered_probes;
    mod renderer_isolation;
    mod serving;
    mod tcp_compatibility;
}
