//! `batter-flags`, the operator command for the flags control plane.
//!
//! It lists, inspects, sets, kills, archives, restores and audits flags and
//! reads the fleet, over one connection with one end-to-end timeout and one
//! attempt. Its endpoint is `BATTER_FLAGS_DATABASE_URL`, captured once;
//! ambient `PG*` settings are refused. Exit codes: 0 ok, 1 failed with
//! nothing written, 2 usage, 3 precondition not met, 4 commit outcome
//! unknown, 101 panic. Under `--output json` every exit writes exactly one
//! JSON document, except the argument parser's own exits (a usage error,
//! `--help` and `--version`) and a panic.
//!
//! As a root that consumes Batter, the first statement of `main` installs
//! Batter's payload-free `redacting_panic_hook`.
#![forbid(unsafe_code)]
#![deny(
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    clippy::unwrap_used
)]

mod capture;
mod commands;
mod endpoint;
mod exit;
mod fleet;
mod output;
mod plan;
mod writes;

use std::{
    io::{self, Write},
    process::ExitCode,
    time::Duration,
};

use batter_flags_postgres::{PooledStore, StoreTiming};
use clap::Parser;
use sqlx::postgres::PgPoolOptions;
use tokio::{
    signal::unix::{SignalKind, signal},
    sync::oneshot,
    time::Instant,
};

use crate::{
    commands::Command,
    endpoint::Endpoint,
    exit::Exit,
    output::{Output, Report},
};

/// Operator command for the batter-flags control plane.
#[derive(Debug, Parser)]
#[command(name = "batter-flags", version)]
struct Cli {
    /// The end-to-end timeout in seconds, 1 to 60.
    #[arg(long, default_value_t = 10, value_parser = clap::value_parser!(u64).range(1..=60))]
    timeout: u64,
    /// How the report is written.
    #[arg(long, value_enum, default_value_t = Output::Text)]
    output: Output,
    #[command(subcommand)]
    command: Command,
}

fn main() -> ExitCode {
    std::panic::set_hook(batter_core::redacting_panic_hook());
    let cli = Cli::parse();
    let output = cli.output;
    let report = run(cli);
    let rendered = report.render(output);
    // Ignored: nothing further can be reported when the stream is closed.
    let _ = match Exit::from_code(report.exit) {
        Exit::Ok => writeln!(io::stdout(), "{rendered}"),
        _ => match output {
            Output::Json => writeln!(io::stdout(), "{rendered}"),
            Output::Text => writeln!(io::stderr(), "{rendered}"),
        },
    };
    ExitCode::from(report.exit)
}

fn run(cli: Cli) -> Report {
    let name = cli.command.name();
    let plan = match plan::plan(cli.command) {
        Ok(plan) => plan,
        Err(message) => return Report::new(name, Exit::Usage, "usage", message),
    };
    let captured = match capture::capture(std::env::vars_os()) {
        Ok(captured) => captured,
        Err(error) => return Report::new(name, Exit::Usage, "usage", error.to_string()),
    };
    let endpoint = match Endpoint::parse(captured.url()) {
        Ok(endpoint) => endpoint,
        Err(error) => return Report::new(name, Exit::Usage, "usage", error.to_string()),
    };
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(_) => return Report::new(name, Exit::Failed, "failed", "the runtime could not start"),
    };
    let timeout = Duration::from_secs(cli.timeout);
    runtime.block_on(async move {
        let deadline = Instant::now() + timeout;
        let Ok(timing) = StoreTiming::new(timeout, timeout, timeout, timeout) else {
            return Report::new(name, Exit::Usage, "usage", "the timeout is out of range");
        };
        let pool = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(timeout)
            .connect_lazy_with(endpoint.options());
        let store = PooledStore::new(pool.clone(), timing);
        let (interrupt, interrupted) = oneshot::channel();
        let (Ok(mut terminate), Ok(mut int)) = (
            signal(SignalKind::terminate()),
            signal(SignalKind::interrupt()),
        ) else {
            return Report::new(
                name,
                Exit::Failed,
                "failed",
                "signal handlers could not be installed",
            );
        };
        tokio::spawn(async move {
            tokio::select! {
                _ = terminate.recv() => {},
                _ = int.recv() => {},
            }
            let _ = interrupt.send(());
        });
        let report = match store.preflight(deadline).await {
            Ok(()) => commands::run(plan, &store, deadline, interrupted).await,
            Err(error) => writes::preflight_failed(name, &error),
        };
        pool.close().await;
        report
    })
}

impl Exit {
    fn from_code(code: u8) -> Self {
        match code {
            0 => Self::Ok,
            2 => Self::Usage,
            3 => Self::Precondition,
            4 => Self::Uncertain,
            _ => Self::Failed,
        }
    }
}
