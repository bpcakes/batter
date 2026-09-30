#[path = "../../../test-support/capture.rs"]
mod capture;

mod operational {
    mod admitted_request;
    mod connect_info;
    mod correlation;
    mod readiness;
    mod rendered_probes;
    mod renderer_isolation;
    mod serving;
    mod tcp_compatibility;
}
