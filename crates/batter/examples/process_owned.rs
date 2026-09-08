//! Finite process-owned work survives a lost request waiter; shutdown cleanup
//! survives a lost shutdown waiter. This is not durable work across process death.
mod support;

use batter::{
    BoxError,
    admission::{Admission, Bulkhead},
    lifecycle::Supervisor,
    operation::OperationContext,
};
use std::{convert::Infallible, time::Duration};
use tokio::sync::oneshot;

#[derive(Debug)]
enum Denial {
    Quota,
}

#[tokio::main]
async fn main() -> Result<(), BoxError> {
    tracing_subscriber::fmt().with_target(false).try_init()?;
    let mut supervisor = Supervisor::with_process_capacity(support::shutdown_budget(), 2)?;
    support::register_signals(&mut supervisor)?;
    let process = supervisor
        .process_handle()
        .expect("capacity was configured");
    supervisor.on_cleanup("dependency.close", || async {
        tracing::info!("dependency closed after admitted work");
        Ok(())
    })?;
    let handle = supervisor.handle();
    handle.mark_ready();
    let running = supervisor.start();
    // Keep body errors inside this block so they cannot return from main and
    // destroy the runtime before the separately owned cleanup has been awaited.
    let submitted = async {
        handle
            .wait_ready()
            .await
            .map_err(|_| std::io::Error::other("startup failed"))?;

        // Transfer the provider permit into owned work. A lost HTTP receipt must
        // not release capacity while the provider call is running.
        let bulkhead = Bulkhead::new(1)?;
        let request = OperationContext::new(Duration::from_secs(1))?;
        let permit = bulkhead.enter(&request, Admission::Reject).await?;
        let (finished, finished_rx) = oneshot::channel();
        let receipt = process.try_spawn("provider.refresh", move |_scope| async move {
            let _permit = permit;
            tokio::time::sleep(Duration::from_millis(20)).await;
            let _ = finished.send(());
            Ok::<_, Infallible>(())
        })?;
        drop(receipt); // A disconnected caller is not the operation's owner.

        // Normal business rejections are values. Err(E) means a process-task
        // failure, so it must not be used for expected quota denials.
        let denied = process.try_spawn("quota.check", |_scope| async {
            Ok::<_, Infallible>(Err::<(), _>(Denial::Quota))
        })?;
        assert!(matches!(denied.wait().await?, Err(Denial::Quota)));
        Ok::<_, BoxError>(finished_rx)
    }
    .await;

    // A separate observer retains the result even after the
    // final driver owner disappears. Last-owner drop requests graceful shutdown.
    let observer = running.observer();
    drop(running);
    let shutdown = observer.wait().await;
    match (submitted, shutdown) {
        (Ok(finished_rx), Ok(report)) if report.is_success() => {
            finished_rx.await?;
            assert_eq!(report.completed_process_tasks, 2);
            Ok(())
        }
        (body, shutdown) => {
            // Both outcomes remain available here for an application-selected
            // trusted sink. Do not automatically print raw causes via Debug.
            tracing::warn!(
                body_failed = body.is_err(),
                shutdown_failed = !shutdown.as_ref().is_ok_and(|report| report.is_success()),
                "example did not complete successfully"
            );
            Err(std::io::Error::other("example operation or shutdown failed").into())
        }
    }
}
