//! Runnable command-only reference service. Provider execution is added later.

use batter::{BoxError, settings::SettingsSource};
use batter_example_reference_service::{
    config::{ConfigMode, RootSettings},
    runtime,
};
use std::{ffi::OsString, path::PathBuf, process::ExitCode};

#[derive(Debug, thiserror::Error)]
#[error("usage: batter-example-reference-service [settings-file]")]
struct ArgumentsError;

fn selected_settings_file(
    arguments: impl IntoIterator<Item = OsString>,
) -> Result<Option<PathBuf>, ArgumentsError> {
    let mut arguments = arguments.into_iter();
    let _binary = arguments.next();
    let selected = arguments.next().map(PathBuf::from);
    if arguments.next().is_some() {
        return Err(ArgumentsError);
    }
    Ok(selected)
}

#[tokio::main]
async fn main() -> ExitCode {
    report_exit(run().await)
}

async fn run() -> Result<(), BoxError> {
    let file = selected_settings_file(std::env::args_os())?;
    let settings = RootSettings::from_process(
        ConfigMode::Serve,
        file.as_deref(),
        SettingsSource::default(),
    )?;
    runtime::run(settings).await
}

fn report_exit(result: Result<(), BoxError>) -> ExitCode {
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            if let Some(error) = error.downcast_ref::<batter::settings::SettingsError>() {
                eprintln!("Error: configuration failed: {error}");
            } else if error.downcast_ref::<ArgumentsError>().is_some() {
                eprintln!("Error: invalid command arguments");
            } else {
                eprintln!("Error: reference service failed");
            }
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn optional_settings_path_is_the_only_command_argument() {
        assert_eq!(
            selected_settings_file([OsString::from("reference-service")]).unwrap(),
            None
        );
        assert_eq!(
            selected_settings_file([
                OsString::from("reference-service"),
                OsString::from("settings.env"),
            ])
            .unwrap(),
            Some(PathBuf::from("settings.env"))
        );
        assert!(
            selected_settings_file([
                OsString::from("reference-service"),
                OsString::from("one"),
                OsString::from("two"),
            ])
            .is_err()
        );
    }
}
