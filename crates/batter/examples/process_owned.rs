//! Finite process-owned work survives a lost request waiter; shutdown cleanup
//! survives a lost shutdown waiter. This is not durable work across process death.
mod support;

use batter::{
    BoxError,
    admission::{Admission, Bulkhead, BulkheadCapacity},
    lifecycle::{Fatal, ProcessCapacity, ShutdownFailure, ShutdownSuccess, Supervisor},
    operation::OperationContext,
};
use std::{convert::Infallible, time::Duration};
use tokio::sync::oneshot;

#[derive(Debug)]
enum Denial {
    Quota,
}

/// Both independent failures remain available for a trusted application sink.
struct CompletionFailure {
    body: Option<BoxError>,
    shutdown: Option<ShutdownFailure>,
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

fn complete(
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

#[tokio::main]
async fn main() -> Result<(), BoxError> {
    tracing_subscriber::fmt().with_target(false).try_init()?;
    let mut supervisor =
        Supervisor::with_process_capacity(support::shutdown_budget(), ProcessCapacity::new(2)?);
    support::register_signals(&mut supervisor)?;
    let process = supervisor
        .process_handle()
        .expect("capacity was configured");
    supervisor.on_cleanup("dependency.close", || async {
        tracing::info!("dependency closed after admitted work");
        Ok(())
    })?;
    let handle = supervisor.handle();
    let running = supervisor.start();
    // Keep body errors inside this block so they cannot return from main and
    // destroy the runtime before the separately owned cleanup has been awaited.
    let submitted = async {
        handle
            .status()
            .wait_ready()
            .await
            .map_err(|_| std::io::Error::other("startup failed"))?;

        // Transfer the provider permit into owned work. A lost HTTP receipt must
        // not release capacity while the provider call is running.
        let bulkhead = Bulkhead::new(BulkheadCapacity::new(1)?);
        let request = OperationContext::new(Duration::from_secs(1))?;
        let permit = bulkhead.enter(&request, Admission::Reject).await?;
        let (finished, finished_rx) = oneshot::channel();
        let receipt = process.try_spawn("provider.refresh", move |_scope| async move {
            let _permit = permit;
            tokio::time::sleep(Duration::from_millis(20)).await;
            let _ = finished.send(());
            Ok::<_, Fatal<Infallible>>(())
        })?;
        drop(receipt); // A disconnected caller is not the operation's owner.

        // Normal business rejections are values. Only `Err(Fatal(_))` is a
        // process-task failure, and a plain `?` cannot produce it.
        let denied = process.try_spawn("quota.check", |_scope| async {
            Ok::<_, Fatal<Infallible>>(Err::<(), _>(Denial::Quota))
        })?;
        assert!(matches!(denied.wait().await?, Err(Denial::Quota)));
        Ok::<_, BoxError>(finished_rx)
    }
    .await;

    // A separate observer retains the result even after the
    // final driver owner disappears. Last-owner drop requests graceful shutdown.
    let observer = running.observer();
    drop(running);
    let shutdown = observer.wait_checked().await;
    let body = match submitted {
        Ok(finished_rx) => finished_rx
            .await
            .map_err(|error| Box::new(error) as BoxError),
        Err(error) => Err(error),
    };
    let success = complete(body, shutdown)?;
    assert_eq!(success.report().completed_process_tasks, 2);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use batter::cleanup::CleanupOutcome;

    #[tokio::test]
    async fn operation_and_shutdown_failures_remain_independently_inspectable() {
        let mut supervisor = Supervisor::new(support::shutdown_budget());
        supervisor
            .register("component", |startup| async move {
                let shutdown = startup.acknowledge_started();
                shutdown.draining().await;
                Ok(shutdown.stopped())
            })
            .unwrap();
        supervisor
            .on_cleanup("resource", || async {
                Err(std::io::Error::other("cleanup-marker").into())
            })
            .unwrap();
        let shutdown = supervisor.start().shutdown_checked().await;
        let body = Err(std::io::Error::other("body-marker").into());
        let error = complete(body, shutdown).unwrap_err();
        let combined = error.downcast_ref::<CompletionFailure>().unwrap();
        assert_eq!(
            combined
                .body
                .as_ref()
                .unwrap()
                .downcast_ref::<std::io::Error>()
                .unwrap()
                .to_string(),
            "body-marker"
        );
        let Some(ShutdownFailure::Report(report)) = &combined.shutdown else {
            panic!("failed shutdown must retain its report")
        };
        assert_eq!(report.cleanup.records[0].outcome, CleanupOutcome::Failed);
        assert_eq!(
            report.cleanup.records[0]
                .error
                .as_ref()
                .unwrap()
                .to_string(),
            "cleanup-marker"
        );
        assert!(!format!("{combined:?}").contains("body-marker"));
        assert!(!format!("{combined:?}").contains("cleanup-marker"));
    }
}
