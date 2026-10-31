use super::*;
use batter_core::{
    command::{Command, CommandCause},
    operation::Interruption,
};

#[tokio::test(flavor = "current_thread")]
async fn command_owner_loss_preserves_dispatch_through_work_and_cleanup_destruction() {
    let ambient = Logs::new();
    let _ambient = tracing::dispatcher::set_default(&ambient.dispatch);
    let driver = Logs::new();
    let second = Duration::from_secs(1);
    let (started, starting) = oneshot::channel();
    let command = driver.within("command.owner", || {
        Command::new(
            batter_core::operation::OperationOwner::new(second)
                .unwrap()
                .into_context(),
            CleanupBudget::new(second, second, second).unwrap(),
            |scope| {
                Box::pin(async move {
                    let _work = DropMarker {
                        name: "command.work",
                        dropped: None,
                    };
                    let cleanup = DropMarker {
                        name: "command.cleanup",
                        dropped: None,
                    };
                    scope
                        .reserve_cleanup("resource")
                        .unwrap()
                        .register(move || async move {
                            drop(cleanup);
                            Ok(())
                        });
                    started.send(()).unwrap();
                    pending::<()>().await;
                    Ok::<_, std::convert::Infallible>(())
                })
            },
        )
        .start()
    });
    starting.await.unwrap();
    let observer = command.observer();
    drop(command);
    let report = observer.wait().await.unwrap();
    assert!(matches!(
        report.work,
        Err(CommandCause::Interrupted(Interruption::Cancelled))
    ));
    assert!(report.cleanup.as_ref().unwrap().is_success());
    driver.one_event("command.work", "command.owner");
    driver.one_event("command.cleanup", "command.owner");
    assert!(!ambient.text().contains("command.work"));
    assert!(!ambient.text().contains("command.cleanup"));
}

/// Lose the owner while cleanup is still running, so the coordinator's
/// completed report is unclaimed and Tokio destroys it outside every polled
/// future, on this thread under the ambient registry.
async fn unclaimed_report_drops_under_driver_dispatch<T, E>(
    name: &'static str,
    complete: fn(DropMarker) -> Result<T, E>,
) where
    T: Send + 'static,
    E: Send + 'static,
{
    let ambient = Logs::new();
    let _ambient = tracing::dispatcher::set_default(&ambient.dispatch);
    let driver = Logs::new();
    let second = Duration::from_secs(1);
    let (dropped, ended) = oneshot::channel();
    let marker = DropMarker {
        name,
        dropped: Some(dropped),
    };
    let (entered, entering) = oneshot::channel();
    let (release, released) = oneshot::channel();
    let owner = driver.within("command.owner", || {
        Command::new(
            batter_core::operation::OperationOwner::new(second * 10)
                .unwrap()
                .into_context(),
            CleanupBudget::new(second * 10, second * 10, second).unwrap(),
            move |scope| {
                Box::pin(async move {
                    scope
                        .reserve_cleanup("resource")
                        .unwrap()
                        .register(move || async move {
                            entered.send(()).unwrap();
                            released.await.unwrap();
                            Ok(())
                        });
                    complete(marker)
                })
            },
        )
        .start_without_signals()
    });
    // Work has returned the marker into its report; cleanup is waiting.
    entering.await.unwrap();
    drop(owner);
    release.send(()).unwrap();
    tokio::time::timeout(second * 10, ended)
        .await
        .expect("the unclaimed report is destroyed")
        .unwrap();
    let event = driver.one_event(&format!("marker=\"{name}\""), "command.owner");
    assert!(event.contains("owned capture dropped"), "{event}");
    assert!(!ambient.text().contains(name), "{}", ambient.text());
}

#[tokio::test(flavor = "current_thread")]
async fn unclaimed_work_value_drops_under_driver_dispatch_after_owner_loss() {
    unclaimed_report_drops_under_driver_dispatch(
        "command.value",
        Ok::<DropMarker, std::convert::Infallible>,
    )
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn unclaimed_work_error_drops_under_driver_dispatch_after_owner_loss() {
    unclaimed_report_drops_under_driver_dispatch("command.error", Err::<(), DropMarker>).await;
}
