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
    let handle = supervisor.handle();
    supervisor.register("signals", move |shutdown| async move {
        #[cfg(unix)]
        {
            use tokio::signal::unix::{SignalKind, signal};
            // Failure to install a listener is an observed critical-task error.
            let mut term = signal(SignalKind::terminate())?;
            let mut interrupt = signal(SignalKind::interrupt())?;
            shutdown.mark_started();
            tokio::select! {
                _ = shutdown.draining() => {},
                _ = term.recv() => handle.request(),
                _ = interrupt.recv() => handle.request(),
            }
        }
        #[cfg(not(unix))]
        {
            use std::{future::Future, task::Poll};
            // Poll registration once before acknowledging startup: merely
            // constructing ctrl_c() does not install the runtime listener.
            let interrupt = tokio::signal::ctrl_c();
            tokio::pin!(interrupt);
            let initial = std::future::poll_fn(|cx| {
                Poll::Ready(match interrupt.as_mut().poll(cx) {
                    Poll::Ready(result) => Some(result),
                    Poll::Pending => None,
                })
            })
            .await;
            shutdown.mark_started();
            if let Some(result) = initial {
                result?;
                handle.request();
                return Ok(());
            }
            tokio::select! {
                _ = shutdown.draining() => {},
                result = &mut interrupt => {
                    result?;
                    handle.request();
                }
            }
        }
        Ok(())
    })?;
    Ok(())
}
