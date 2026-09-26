use super::*;

pub fn observation_budget() -> Duration {
    REAP_LIMIT * 2
}

/// Pair typed startup/cleanup evidence with the actual executable's exit contract.
pub fn probe(endpoint: &str) -> Result<(), String> {
    let forbidden = super::super::provider_forbidden_values(endpoint, "http://127.0.0.1:9/");
    StartupChild::start("announcement.failure", endpoint)
        .map_err(|_| "announcement assertion child could not start")?
        .finish(&[], &forbidden)?;

    let path = missing_path()?;
    let mut command = Command::new(env!("CARGO_BIN_EXE_batter-example-reference-service"));
    configure(&mut command, endpoint, "127.0.0.1:0");
    command
        .env("BATTER_LISTENER_ANNOUNCEMENT_PATH", &path)
        .stdin(Stdio::null());
    let child = SeparateChild::spawn("missing announcement receiver", command, None)
        .map_err(|_| "production announcement child could not start")?;
    let mut forbidden = forbidden.to_vec();
    forbidden.push(&path);
    // No signal: natural startup failure must terminate and reap within the bound.
    child.wait(REAP_LIMIT)?.production_child(&forbidden)
}

fn missing_path() -> Result<String, String> {
    let path = format!("/tmp/batter-missing-{}", uuid::Uuid::now_v7());
    match std::fs::symlink_metadata(&path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(path),
        _ => Err("missing-receiver setup did not select an absent path".into()),
    }
}

pub async fn child() {
    use batter::{
        cleanup::CleanupOutcome,
        settings::SettingsSource,
        startup::{InitializationError, StartupCause, StartupError},
    };
    use batter_example_reference_service::{
        config::ServingSettings,
        runtime::{self, ProtectedRuntimeStartupFailure},
    };
    use std::error::Error;

    let settings = ServingSettings::from_process(
        None,
        SettingsSource::from_pairs([(
            "BATTER_LISTENER_ANNOUNCEMENT_PATH".into(),
            missing_path().expect("absent receiver path").into(),
        )])
        .expect("valid override"),
    )
    .expect("valid announcement settings");
    let completion = runtime::run(runtime::prepare(settings).expect("inert preparation")).await;
    let failure = completion
        .service()
        .expect_err("missing receiver must fail startup")
        .error()
        .downcast_ref::<ProtectedRuntimeStartupFailure>()
        .expect("protected startup failure");
    let StartupError::Failed(report) = failure.startup() else {
        panic!("startup coordinator failed");
    };
    assert_eq!(report.stage, "http.bind");
    let StartupCause::Failed(InitializationError::Application(primary)) = &report.cause else {
        panic!("announcement must be an application initialization failure");
    };
    assert_eq!(
        primary
            .source()
            .and_then(|source| source.downcast_ref::<io::Error>())
            .map(io::Error::kind),
        Some(io::ErrorKind::NotFound),
        "retain the missing Unix receiver cause, not an unrelated startup failure"
    );
    assert!(report.destruction_panic.is_none());
    assert!(report.cleanup.skipped.is_empty());
    assert_eq!(report.cleanup.records.len(), 1);
    assert_eq!(report.cleanup.records[0].name, "postgres.pool");
    assert_eq!(report.cleanup.records[0].outcome, CleanupOutcome::Succeeded);
}
