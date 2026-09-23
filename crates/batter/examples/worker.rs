//! In-memory maintenance task; durable work belongs in Runledger.
mod support;

use batter::{BoxError, lifecycle::Supervisor};
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), BoxError> {
    tracing_subscriber::fmt().with_target(false).try_init()?;
    let mut supervisor = Supervisor::new(support::shutdown_budget());
    support::register_signals(&mut supervisor)?;
    supervisor.register("maintenance", |startup| async move {
        let mut interval = tokio::time::interval(Duration::from_secs(5));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let shutdown = startup.acknowledge_started();
        loop {
            tokio::select! {
                biased;
                _ = shutdown.draining() => break,
                _ = interval.tick() => tracing::info!("maintenance tick"),
            }
        }
        Ok(shutdown.stopped())
    })?;
    let running = supervisor.start();
    running.wait_checked().await?;
    Ok(())
}
