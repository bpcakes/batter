//! Process-only signal witness for the reference integration tests.

use batter::{BoxError, settings::SettingsSource};
use batter_example_reference_service::{config::ServingSettings, runtime};
use std::{io::Write, process::ExitCode};
use tokio::signal::unix::{SignalKind, signal};

const SIGTERM_OBSERVED: &str = "batter-fixture:executable-sigterm-observed";
const SIGINT_OBSERVED: &str = "batter-fixture:executable-sigint-observed";

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => {
            eprintln!("Error: reference service failed");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), BoxError> {
    let settings = ServingSettings::from_process(None, SettingsSource::default())?;
    let prepared = runtime::prepare(settings)?;
    let mut terminate = signal(SignalKind::terminate())?;
    let mut interrupt = signal(SignalKind::interrupt())?;
    let application = runtime::run(prepared);
    tokio::pin!(application);
    let event = tokio::select! {
        biased;
        received = terminate.recv() => {
            received.ok_or_else(|| std::io::Error::other("SIGTERM witness closed"))?;
            SIGTERM_OBSERVED
        }
        received = interrupt.recv() => {
            received.ok_or_else(|| std::io::Error::other("SIGINT witness closed"))?;
            SIGINT_OBSERVED
        }
        result = &mut application => return result,
    };

    // The parent signals only after the withheld native handshake is accepted,
    // which proves startup installed its own listeners. Await application cleanup
    // before touching output so a broken capture cannot discard the runtime owner.
    let application = application.await;
    let mut stdout = std::io::stdout().lock();
    writeln!(stdout, "{event}")?;
    stdout.flush()?;
    application
}
