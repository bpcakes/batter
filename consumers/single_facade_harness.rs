//! Test harness for `single_facade_consumer.rs`. It is deliberately a separate
//! program: the acceptance consumer must prove it needs no test support, so the
//! disposable PostgreSQL 18 database is provisioned here. It reaches Runledger's
//! existing native test support through the facade's `runledger-test-support`
//! feature, so this program also declares only `batter`.
//!
//! Usage: `single-facade-harness <consumer-binary>`. The harness creates an
//! ephemeral database, runs the consumer with `DATABASE_URL` pointing at it,
//! forwards the consumer's output, drops the database and reports the
//! consumer's status. Docker provisioning and teardown remain Runledger's.

use batter::runledger::native::test_support::create_ephemeral_database;
use std::{path::PathBuf, process::Command};

type Outcome<T = ()> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[tokio::main]
async fn main() -> Outcome {
    let consumer = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or("usage: single-facade-harness <consumer-binary>")?;
    let database = create_ephemeral_database("facade_consumer").await?;
    // Retain the consumer's status so teardown still runs before reporting it.
    let status = Command::new(&consumer)
        .env("DATABASE_URL", database.url())
        .env("FACADE_CONSUMER_SUBJECT", database.name())
        .status();
    let teardown = database.teardown().await;
    let status = status?;
    teardown?;
    if !status.success() {
        // Only the status reaches diagnostics; the consumer owns its own output.
        return Err(format!("single-facade consumer exited with {status}").into());
    }
    println!("single-facade harness: disposable database created, consumed and dropped");
    Ok(())
}
