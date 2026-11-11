//! A checkout service: an HTTP route group and a receipt job, made
//! flag-aware through Batter's typed flags in control-plane mode.
//!
//! - `checkout-service` (or `checkout-service serve`) serves `POST /checkout`
//!   and `GET /checkout/status` in the `checkout` route group, runs the
//!   `receipt.send` worker and keeps the flags snapshot fresh from the audited
//!   PostgreSQL store, starting from the `FLAG_*` settings.
//! - `checkout-service migrate` applies Runledger's, this service's and the
//!   flags store's migrations under the schema-owner login.
//!
//! Settings are read once, here, from the process environment; see
//! `src/config.rs` and `src/flags.rs` for the names. Operators engage the
//! `checkout_halt` kill switch with `ops/engage-halt.sh`.

mod config;
mod flags;
mod http;
mod migrate;
mod orders;
mod receipts;
mod serve;

use std::process::ExitCode;

use batter::{BoxError, settings::SettingsSource};

enum Mode {
    Serve,
    Migrate,
}

fn mode(arguments: &[String]) -> Option<Mode> {
    match arguments {
        [] => Some(Mode::Serve),
        [command] if command == "serve" => Some(Mode::Serve),
        [command] if command == "migrate" => Some(Mode::Migrate),
        _ => None,
    }
}

async fn root(arguments: Vec<String>) -> Result<(), BoxError> {
    let Some(mode) = mode(&arguments) else {
        return Err("usage: checkout-service [serve|migrate]".into());
    };
    // The one read of the process environment.
    let captured = SettingsSource::from_pairs(std::env::vars_os())?;
    match mode {
        Mode::Serve => {
            // Flags resolve from the unnarrowed capture, before the
            // application narrows its own names.
            let (resolved, handles) = flags::resolve(&captured, flags::today()?)?;
            let config = config::ServeConfig::from_settings(&captured)?;
            println!("checkout-service: {}", resolved.report());
            serve::run(config, resolved, handles).await
        }
        Mode::Migrate => migrate::run(config::MigrateConfig::from_settings(&captured)?).await,
    }
}

fn main() -> ExitCode {
    std::panic::set_hook(batter::redacting_panic_hook());
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let Ok(runtime) = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    else {
        eprintln!("checkout-service: the runtime could not start");
        return ExitCode::FAILURE;
    };
    let outcome = runtime.block_on(root(arguments));
    // A last-good persist the drain abandoned may still be writing; bound it.
    runtime.shutdown_timeout(std::time::Duration::from_secs(5));
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            // Batter, settings and flags errors format without values or causes.
            eprintln!("checkout-service: {error}");
            ExitCode::FAILURE
        }
    }
}
