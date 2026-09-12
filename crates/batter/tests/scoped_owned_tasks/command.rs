use super::*;
use batter::{
    command::{Command, CommandCause},
    operation::{Interruption, OperationContext},
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
            OperationContext::new(second).unwrap(),
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
