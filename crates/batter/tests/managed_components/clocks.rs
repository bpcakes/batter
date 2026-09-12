use crate::managed_support::{NativeReport, context, supervisor};
use batter::lifecycle::ManagedComponent;
use std::{future::pending, time::Duration};
use tokio::{sync::oneshot, time::Instant};

#[tokio::test(start_paused = true)]
async fn delayed_native_stop_uses_its_original_time_for_process_escalation() {
    let mut process = supervisor();
    process
        .on_cleanup("dependency", || async {
            panic!("unsettled native work must block cleanup")
        })
        .unwrap();
    let (stopped, stopping) = oneshot::channel();
    let native_started = Instant::now();
    process
        .register_managed("native", context(), move |_| {
            Ok(ManagedComponent::new(
                async { Ok(()) },
                async move {
                    stopping.await.unwrap();
                },
                move |parent_started| native_started.min(parent_started),
                pending::<NativeReport>(),
            ))
        })
        .unwrap();
    let running = process.start();
    running.handle().mark_ready();
    running.handle().wait_ready().await.unwrap();
    tokio::time::advance(Duration::from_secs(2)).await;
    stopped.send(()).unwrap();
    let report = running.wait().await.unwrap();
    assert_eq!(Instant::now(), native_started + Duration::from_secs(3));
    assert!(!report.is_success());
    assert!(!report.managed[0].outcome.allows_dependency_cleanup());
    assert_eq!(report.cleanup.skipped.len(), 1);
    assert_eq!(
        report.cleanup.skipped[0].reason,
        batter::cleanup::SkipReason::UnsafeTaskExit
    );
}
