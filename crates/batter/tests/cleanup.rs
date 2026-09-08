use batter::{
    BoxError,
    cleanup::{CleanupBudget, CleanupOutcome, CleanupStack, SkipReason},
};
use std::{
    future::pending,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

fn budget() -> CleanupBudget {
    CleanupBudget::new(
        Duration::from_secs(5),
        Duration::from_secs(1),
        Duration::from_millis(100),
    )
    .unwrap()
}

struct Dropped(Arc<AtomicBool>);
impl Drop for Dropped {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

#[tokio::test(start_paused = true)]
async fn finalizers_run_in_reverse_registration_order() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut cleanup = CleanupStack::new();
    for name in ["pool", "client", "exporter"] {
        let events = events.clone();
        cleanup
            .push(name, move || async move {
                events.lock().unwrap().push(name);
                Ok(())
            })
            .unwrap();
    }
    let report = cleanup.close(budget()).await;
    assert!(report.is_success());
    assert_eq!(*events.lock().unwrap(), vec!["exporter", "client", "pool"]);
}

#[tokio::test(start_paused = true)]
async fn all_returned_errors_are_retained() {
    let mut cleanup = CleanupStack::new();
    for name in ["first", "second"] {
        cleanup
            .push(name, move || async move {
                Err(std::io::Error::other(name).into())
            })
            .unwrap();
    }
    let report = cleanup.close(budget()).await;
    assert!(!report.is_success());
    assert_eq!(report.records.len(), 2);
    assert!(
        report
            .records
            .iter()
            .all(|record| record.outcome == CleanupOutcome::Failed)
    );
    assert_eq!(
        report.records[0].error.as_ref().unwrap().to_string(),
        "second"
    );
    assert_eq!(
        report.records[1].error.as_ref().unwrap().to_string(),
        "first"
    );
}

#[tokio::test(start_paused = true)]
async fn cleanup_panic_is_retained_and_later_hooks_still_run() {
    let mut cleanup = CleanupStack::new();
    cleanup.push("last", || async { Ok(()) }).unwrap();
    cleanup
        .push("panicking", || async {
            panic!("cleanup defect");
            #[allow(unreachable_code)]
            Ok(())
        })
        .unwrap();
    let report = cleanup.close(budget()).await;
    assert_eq!(report.records[0].outcome, CleanupOutcome::Panicked);
    assert_eq!(report.records[1].outcome, CleanupOutcome::Succeeded);
}

#[tokio::test(start_paused = true)]
async fn synchronous_factory_panic_is_inside_the_task_boundary() {
    let mut cleanup = CleanupStack::new();
    cleanup
        .push("factory", || -> std::future::Ready<Result<(), BoxError>> {
            panic!("factory panic")
        })
        .unwrap();
    let report = cleanup.close(budget()).await;
    assert_eq!(report.records[0].outcome, CleanupOutcome::Panicked);
}

#[tokio::test(start_paused = true)]
async fn timed_out_hook_is_dropped_before_next_dependency_hook() {
    let dropped = Arc::new(AtomicBool::new(false));
    let mut cleanup = CleanupStack::new();
    let check = dropped.clone();
    cleanup
        .push("dependency", move || async move {
            assert!(check.load(Ordering::SeqCst));
            Ok(())
        })
        .unwrap();
    let flag = dropped.clone();
    cleanup
        .push("stuck", move || async move {
            let _guard = Dropped(flag);
            pending::<Result<(), BoxError>>().await
        })
        .unwrap();
    let report = cleanup.close(budget()).await;
    assert_eq!(report.records[0].outcome, CleanupOutcome::TimedOut);
    assert_eq!(report.records[1].outcome, CleanupOutcome::Succeeded);
    assert!(dropped.load(Ordering::SeqCst));
}

#[tokio::test(start_paused = true)]
async fn total_work_budget_is_shared_and_skipped_hooks_are_explicit() {
    let mut cleanup = CleanupStack::new();
    cleanup
        .push("never-started", || async {
            panic!("must be skipped");
            #[allow(unreachable_code)]
            Ok(())
        })
        .unwrap();
    cleanup
        .push("uses-budget", pending::<Result<(), BoxError>>)
        .unwrap();
    let tiny = CleanupBudget::new(
        Duration::from_millis(5),
        Duration::from_secs(1),
        Duration::from_millis(1),
    )
    .unwrap();
    let report = cleanup.close(tiny).await;
    assert_eq!(report.records[0].outcome, CleanupOutcome::TimedOut);
    assert_eq!(report.skipped[0].name, "never-started");
    assert_eq!(report.skipped[0].reason, SkipReason::BudgetExhausted);
}

#[test]
fn dropping_the_stack_does_not_pretend_to_run_async_cleanup() {
    let started = Arc::new(AtomicBool::new(false));
    let flag = started.clone();
    let mut cleanup = CleanupStack::new();
    cleanup
        .push("not-run", move || {
            flag.store(true, Ordering::SeqCst);
            async { Ok(()) }
        })
        .unwrap();
    drop(cleanup);
    assert!(!started.load(Ordering::SeqCst));
}

#[test]
fn explicit_skip_does_not_start_hooks() {
    let mut cleanup = CleanupStack::new();
    cleanup
        .push("not-run", || -> std::future::Ready<Result<(), BoxError>> {
            panic!("must not start")
        })
        .unwrap();
    let report = cleanup.skip(SkipReason::UnsafeTaskExit);
    assert_eq!(report.skipped.len(), 1);
    assert!(!report.is_success());
}

#[test]
fn registration_rejects_duplicates_and_unbounded_labels() {
    let mut cleanup = CleanupStack::new();
    cleanup.push("pool", || async { Ok(()) }).unwrap();
    assert!(cleanup.push("pool", || async { Ok(()) }).is_err());
    assert!(
        cleanup
            .push("email@example.test", || async { Ok(()) })
            .is_err()
    );
    let _ = cleanup.skip(SkipReason::UnsafeTaskExit);
}

#[test]
fn cleanup_budget_is_validated() {
    assert!(
        CleanupBudget::new(
            Duration::ZERO,
            Duration::from_secs(1),
            Duration::from_secs(1)
        )
        .is_err()
    );
    assert!(
        CleanupBudget::new(
            Duration::MAX,
            Duration::from_secs(1),
            Duration::from_secs(1)
        )
        .is_err()
    );
}

#[tokio::test(start_paused = true)]
async fn empty_cleanup_is_successful() {
    assert!(CleanupStack::new().close(budget()).await.is_success());
}

#[tokio::test(start_paused = true)]
async fn dropping_close_aborts_the_active_hook_without_starting_dependencies() {
    struct NotifyDrop(Option<tokio::sync::oneshot::Sender<()>>);
    impl Drop for NotifyDrop {
        fn drop(&mut self) {
            let _ = self.0.take().unwrap().send(());
        }
    }

    let dependency_started = Arc::new(AtomicBool::new(false));
    let dependency = dependency_started.clone();
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (dropped_tx, dropped_rx) = tokio::sync::oneshot::channel();
    let mut cleanup = CleanupStack::new();
    cleanup
        .push("dependency", move || {
            dependency.store(true, Ordering::SeqCst);
            async { Ok(()) }
        })
        .unwrap();
    cleanup
        .push("active", move || async move {
            let _guard = NotifyDrop(Some(dropped_tx));
            started_tx.send(()).unwrap();
            pending::<Result<(), BoxError>>().await
        })
        .unwrap();

    let close = tokio::spawn(cleanup.close(budget()));
    started_rx.await.unwrap();
    close.abort();
    assert!(close.await.unwrap_err().is_cancelled());
    tokio::time::timeout(Duration::from_secs(1), dropped_rx)
        .await
        .expect("the directly owned finalizer must be aborted")
        .unwrap();
    assert!(!dependency_started.load(Ordering::SeqCst));
}
