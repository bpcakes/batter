use runledger_runtime::{
    RuntimeShutdownBudget, RuntimeShutdownReport, RuntimeShutdownSignal, Supervisor,
};

/// Observe initialization while the native owner drives shutdown independently.
/// This announces local loop initialization, not database health or job success.
pub async fn run(
    supervisor: Supervisor,
    signal: RuntimeShutdownSignal,
    budget: RuntimeShutdownBudget,
    initialized: impl FnOnce(),
) -> RuntimeShutdownReport {
    let startup = supervisor.startup_observer();
    let mut driver = supervisor.run_until_shutdown_report(signal, budget);
    tokio::select! {
        biased;
        report = &mut driver => report,
        state = startup.wait_initialized() => {
            if state.is_ok() {
                initialized();
            }
            driver.await
        }
    }
}
