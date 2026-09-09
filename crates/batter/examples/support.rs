use batter::{
    BoxError,
    cleanup::CleanupBudget,
    lifecycle::{ShutdownBudget, Supervisor},
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

pub fn register_signals(supervisor: &mut Supervisor) -> Result<(), BoxError> {
    batter::lifecycle::register_signals(supervisor, "signals")?;
    Ok(())
}
