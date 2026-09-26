//! Global metrics installation and owned finalization in isolated Unix children.
//!
//! Every scenario installs at most one process-wide recorder, so each runs in
//! its own child process with a cleared environment. Children use injected
//! settings, a local collector and a refused or withheld PostgreSQL endpoint;
//! no database is required.

#[path = "support/collector.rs"]
mod collector;
#[path = "support/configuration_process.rs"]
mod process_runtime;

mod metrics_export {
    pub(crate) mod children;
    mod executable;
}

use metrics_export::children;
use process_runtime::{ExpectedExit, run};

const COMPLETE: &str = "metrics-child:complete";
/// Credential embedded in every child's database URL; never printed.
const PRIVATE: &str = "secret-marker";

fn scenario(name: &str) {
    run(name, &[])
        .validate_text(ExpectedExit::Success, &[COMPLETE], &[PRIVATE, "panicked"])
        .unwrap();
}

#[test]
fn child_fixture() {
    if let Some(scenario) = process_runtime::launch::scenario() {
        children::dispatch(&scenario);
    }
}

#[test]
fn startup_failure_cleanup_is_exported_on_both_runtime_flavors() {
    scenario("startup-export-current");
    scenario("startup-export-multi");
}

#[test]
fn rejected_installation_closes_the_pipeline_and_keeps_the_service_result() {
    scenario("rejected-installation");
}

#[test]
fn never_polled_serving_installs_and_contacts_nothing() {
    scenario("never-polled");
}

#[test]
fn waiters_and_owner_loss_leave_setup_cleanup_and_final_export_owned() {
    scenario("owner-loss-current");
    scenario("owner-loss-multi");
}
