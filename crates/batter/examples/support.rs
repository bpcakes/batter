use batter::{
    BoxError,
    cleanup::CleanupBudget,
    lifecycle::{ShutdownBudget, ShutdownFailure, ShutdownSuccess, Supervisor},
};
use std::time::Duration;

pub fn cleanup_budget() -> CleanupBudget {
    CleanupBudget::new(
        Duration::from_secs(5),
        Duration::from_secs(3),
        Duration::from_secs(1),
    )
    .expect("constant cleanup budget is valid")
}

pub fn shutdown_budget() -> ShutdownBudget {
    ShutdownBudget::new(
        Duration::from_secs(10),
        Duration::from_secs(2),
        Duration::from_secs(1),
        cleanup_budget(),
    )
    .expect("constant shutdown budget is valid")
}

#[allow(dead_code, reason = "not every example registers signals")]
pub fn register_signals(supervisor: &mut Supervisor) -> Result<(), BoxError> {
    batter::lifecycle::register_signals(supervisor, "signals")?;
    Ok(())
}

/// Both independent failures remain available for a trusted application sink.
#[allow(
    dead_code,
    reason = "not every example awaits a body beside its shutdown"
)]
pub struct CompletionFailure {
    pub body: Option<BoxError>,
    pub shutdown: Option<ShutdownFailure>,
}

impl std::fmt::Display for CompletionFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("example operation or shutdown failed")
    }
}

impl std::fmt::Debug for CompletionFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self, f)
    }
}

impl std::error::Error for CompletionFailure {}

#[allow(
    dead_code,
    reason = "not every example awaits a body beside its shutdown"
)]
pub fn complete(
    body: Result<(), BoxError>,
    shutdown: Result<ShutdownSuccess, ShutdownFailure>,
) -> Result<ShutdownSuccess, BoxError> {
    match (body, shutdown) {
        (Ok(()), Ok(success)) => Ok(success),
        (body, shutdown) => {
            let failure = CompletionFailure {
                body: body.err(),
                shutdown: shutdown.err(),
            };
            tracing::warn!(
                body_failed = failure.body.is_some(),
                shutdown_failed = failure.shutdown.is_some(),
                "example did not complete successfully"
            );
            Err(Box::new(failure))
        }
    }
}
