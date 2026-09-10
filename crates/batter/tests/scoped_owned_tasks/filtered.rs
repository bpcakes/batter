use super::*;
use std::convert::Infallible;

#[tokio::test(flavor = "current_thread")]
async fn filtered_task_parents_survive_execution_and_destruction_on_current_thread() {
    exercise().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn filtered_task_parents_survive_execution_and_destruction_on_multiple_workers() {
    exercise().await;
}

async fn exercise() {
    let ambient = Logs::new();
    let checks = ambient.within("unrelated", || {
        batter::telemetry::with_current_dispatch(
            async {
                process_tasks(false).await;
                process_tasks(true).await;
                aborted_cleanup().await;
            }
            .in_current_span(),
        )
    });
    tokio::time::timeout(Duration::from_secs(10), checks)
        .await
        .unwrap();
    assert!(!ambient.text().contains("owned capture dropped"));
    assert!(!ambient.text().contains("callback entered"));
}

fn assert_context(logs: &Logs, marker: &str, scope: &str) {
    let entered = logs.one_event(&format!("callback entered marker=\"{marker}\""), scope);
    let dropped = logs.one_event(&format!("owned capture dropped marker=\"{marker}\""), scope);
    for event in [entered, dropped] {
        assert!(!event.contains("batter.task"), "{event}");
        assert!(!event.contains("batter.process_task"), "{event}");
        assert!(!event.contains("batter.cleanup"), "{event}");
        assert!(!event.contains("unrelated"), "{event}");
    }
}

async fn process_tasks(abort: bool) {
    let driver = Logs::with_filter("info,batter=warn");
    let request = Logs::with_filter("info,batter=warn");
    let second = Duration::from_secs(1);
    let budget = if abort {
        shutdown_budget()
    } else {
        ShutdownBudget::new(
            second,
            second,
            second,
            CleanupBudget::new(second, second, second).unwrap(),
        )
        .unwrap()
    };
    let mut supervisor = Supervisor::with_process_capacity(budget, 1).unwrap();
    let process = supervisor.process_handle().unwrap();
    supervisor
        .register("critical.worker", move |signal| async move {
            let _capture = DropMarker {
                name: "critical.capture",
                dropped: None,
            };
            tracing::info!(marker = "critical.capture", "callback entered");
            signal.mark_started();
            if abort {
                pending::<()>().await;
            } else {
                signal.draining().await;
            }
            Ok(())
        })
        .unwrap();
    supervisor
        .on_cleanup("dependency.close", || async {
            let _capture = DropMarker {
                name: "cleanup.capture",
                dropped: None,
            };
            tracing::info!(marker = "cleanup.capture", "callback entered");
            Ok(())
        })
        .unwrap();
    supervisor.handle().mark_ready();
    let running = driver.within("driver", || supervisor.start());
    running.handle().wait_ready().await.unwrap();
    let (started, ready) = oneshot::channel();
    let receipt = request.within("request", || {
        process
            .try_spawn("finite.work", move |_| async move {
                let _capture = DropMarker {
                    name: "finite.capture",
                    dropped: None,
                };
                tracing::info!(marker = "finite.capture", "callback entered");
                started.send(()).unwrap();
                if abort {
                    pending::<()>().await;
                }
                Ok::<_, Infallible>(())
            })
            .unwrap()
    });
    ready.await.unwrap();
    if !abort {
        receipt.wait().await.unwrap();
    } else {
        drop(receipt);
    }
    let report = running.shutdown().await.unwrap();
    assert_eq!(report.is_success(), !abort);
    if abort {
        assert_eq!(report.abort_requested, ["critical.worker", "finite.work"]);
        assert_eq!(report.tasks.len(), 2);
        assert!(
            report
                .tasks
                .iter()
                .all(|task| task.outcome == TaskOutcome::Aborted)
        );
        assert_eq!(report.cleanup.skipped.len(), 1);
        assert!(!driver.text().contains("cleanup.capture"));
    } else {
        assert_eq!(report.completed_process_tasks, 1);
        assert_context(&driver, "cleanup.capture", "driver");
    }
    assert_context(&driver, "critical.capture", "driver");
    assert_context(&request, "finite.capture", "request");
    assert!(!driver.text().contains("finite.capture"));
    assert!(!request.text().contains("critical.capture"));
    assert!(!request.text().contains("cleanup.capture"));
}

async fn aborted_cleanup() {
    let owner = Logs::with_filter("info,batter=warn");
    let mut cleanup = CleanupStack::new();
    let (started, ready) = oneshot::channel();
    let (dropped, ended) = oneshot::channel();
    cleanup
        .push("active.close", || async move {
            let _capture = DropMarker {
                name: "active.capture",
                dropped: Some(dropped),
            };
            tracing::info!(marker = "active.capture", "callback entered");
            started.send(()).unwrap();
            pending::<Result<(), BoxError>>().await
        })
        .unwrap();
    let second = Duration::from_secs(1);
    let close = owner.within("cleanup", || {
        tokio::spawn(batter::telemetry::with_current_dispatch(
            cleanup
                .close(CleanupBudget::new(second, second, second).unwrap())
                .in_current_span(),
        ))
    });
    ready.await.unwrap();
    close.abort();
    assert!(close.await.unwrap_err().is_cancelled());
    tokio::time::timeout(second, ended).await.unwrap().unwrap();
    assert_context(&owner, "active.capture", "cleanup");
}
