//! Offline maintenance command. Deployment revocation remains external.
use batter::{
    BoxError, cleanup::CleanupBudget, command::CommandOutcome, operation::OperationContext,
    settings::SettingsSource,
};
use batter_example_reference_service::{
    config::{ConfigMode, RootSettings},
    retirement::{self, DatabaseIdentity, RetirementError, RetirementReport},
};
use std::{path::PathBuf, process::ExitCode, time::Duration};

#[path = "retire_startup_controls/diagnostics.rs"]
mod diagnostics;

#[tokio::main]
async fn main() -> ExitCode {
    let (facts, success) = match run().await {
        Ok(outcome) => diagnostics::outcome(outcome),
        Err(_) => (diagnostics::setup_failure(), false),
    };
    if success {
        println!("{facts}");
        ExitCode::SUCCESS
    } else {
        eprintln!("{facts}");
        ExitCode::FAILURE
    }
}

async fn run() -> Result<CommandOutcome<RetirementReport, RetirementError>, BoxError> {
    let mut args = std::env::args_os().skip(1);
    let system = args
        .next()
        .and_then(|s| s.into_string().ok())
        .ok_or("expected cluster identifier required")?
        .parse()?;
    let database = args
        .next()
        .and_then(|s| s.into_string().ok())
        .ok_or("expected database OID required")?
        .parse()?;
    let file = args.next().map(PathBuf::from);
    if args.next().is_some() {
        return Err("usage: retire_startup_controls SYSTEM_ID DATABASE_OID [settings-file]".into());
    }
    let expected = DatabaseIdentity::new(system, database)?;
    let settings = RootSettings::from_process(
        ConfigMode::Setup,
        file.as_deref(),
        SettingsSource::default(),
    )?;
    let second = Duration::from_secs(1);
    let command = retirement::prepare(
        settings.connect_options_from_process()?,
        expected,
        OperationContext::new(second * 30)?,
        CleanupBudget::new(second * 3, second * 3, second)?,
    )
    .start();
    Ok(command.wait().await)
}
