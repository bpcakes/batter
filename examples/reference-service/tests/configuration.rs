#[path = "support/configuration_process.rs"]
mod process_runtime;
#[path = "../../../test-support/temp_dir.rs"]
mod temp_dir;
mod configuration {
    mod boundaries;
    mod consumers;
    pub(crate) mod diagnostics;
    mod live_endpoint;
    pub(crate) mod process;
    mod report;
    mod startup;
    mod tracing;
}

use batter::settings::SettingsSource;
use batter_example_reference_service::config::ServingSettings;

fn source(pairs: &[(&str, &str)]) -> SettingsSource {
    SettingsSource::from_pairs(
        pairs
            .iter()
            .map(|(key, value)| ((*key).into(), (*value).into())),
    )
    .unwrap()
}
fn load(pairs: &[(&str, &str)]) -> Result<ServingSettings, batter::settings::SettingsError> {
    let mut values = source(&[
        (
            "DATABASE_URL",
            "postgres://user:fake-password@localhost/database?sslmode=disable",
        ),
        ("JOBS_WORKER_ID", "configured-worker"),
        (
            "BATTER_AUTH_OWNER_ID",
            "00000000-0000-0000-0000-000000000001",
        ),
        ("BATTER_AUTH_TOKEN", "fake-configured-token"),
    ]);
    values.overlay(source(pairs));
    ServingSettings::from_sources(None, SettingsSource::default(), values)
}

#[test]
fn child_fixture() {
    if let Some(scenario) = configuration::process::launch::scenario() {
        configuration::diagnostics::child(&scenario);
    }
}
