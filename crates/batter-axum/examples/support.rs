use batter::{
    BoxError,
    cleanup::CleanupBudget,
    lifecycle::{ShutdownBudget, Supervisor},
};
use std::time::Duration;
use tokio::signal::unix::{SignalKind, signal};

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
    let handle = supervisor.handle();
    supervisor.register("signals", move |shutdown| async move {
        // Failure to install a listener is an observed critical-task error.
        let mut term = signal(SignalKind::terminate())?;
        let mut interrupt = signal(SignalKind::interrupt())?;
        shutdown.mark_started();
        tokio::select! {
            _ = shutdown.draining() => {},
            _ = term.recv() => handle.request(),
            _ = interrupt.recv() => handle.request(),
        }
        Ok(())
    })?;
    Ok(())
}
