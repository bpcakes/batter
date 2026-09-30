use super::*;
use batter::{cleanup::CleanupOutcome, lifecycle::Supervisor};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::oneshot;

#[tokio::test]
async fn work_failure_waits_for_worker_settlement_before_cleanup_and_return() {
    let mut supervisor = Supervisor::new(crate::budget().unwrap());
    let (draining, drain_observed) = oneshot::channel();
    let (release, released) = oneshot::channel();
    let stopped = Arc::new(AtomicBool::new(false));
    let task_stopped = stopped.clone();
    supervisor
        .register("worker", move |startup| async move {
            let shutdown = startup.acknowledge_started();
            shutdown.draining().await;
            draining.send(()).unwrap();
            released.await.unwrap();
            task_stopped.store(true, Ordering::SeqCst);
            Ok(shutdown.stopped())
        })
        .unwrap();
    let closed = Arc::new(AtomicBool::new(false));
    let cleanup_closed = closed.clone();
    supervisor
        .on_cleanup("pool", move || async move {
            assert!(
                stopped.load(Ordering::SeqCst),
                "dependency closed before worker settled"
            );
            cleanup_closed.store(true, Ordering::SeqCst);
            Ok(())
        })
        .unwrap();
    let running = supervisor.start();
    running.status().wait_ready().await.unwrap();
    let completion = tokio::spawn(complete(running, async {
        Err::<(), _>(std::io::Error::other("submission-marker").into())
    }));
    drain_observed.await.unwrap();
    assert!(!completion.is_finished(), "work error bypassed settlement");
    assert!(!closed.load(Ordering::SeqCst));
    release.send(()).unwrap();
    let error = completion.await.unwrap().unwrap_err();
    assert_eq!(
        error.downcast_ref::<std::io::Error>().unwrap().to_string(),
        "submission-marker"
    );
    assert!(closed.load(Ordering::SeqCst));
}

#[tokio::test]
async fn readiness_failure_retains_the_application_error_and_task_report() {
    let mut supervisor = Supervisor::new(crate::budget().unwrap());
    supervisor
        .register("worker", |_startup| async {
            Err(std::io::Error::other("initialization-marker").into())
        })
        .unwrap();
    let running = supervisor.start();
    let status = running.status();
    let error = complete(running, async {
        status.wait_ready().await.map_err(|_| "readiness-marker")?;
        Ok(())
    })
    .await
    .unwrap_err();
    let both = error.downcast_ref::<WorkAndShutdownFailure>().unwrap();
    assert_eq!(both.work.to_string(), "readiness-marker");
    let ShutdownFailure::Report(report) = &both.shutdown else {
        panic!("expected the retained task report")
    };
    assert_eq!(
        report.tasks[0].error.as_ref().unwrap().to_string(),
        "initialization-marker"
    );
    assert!(!format!("{both:?}").contains("marker"));
    assert!(!format!("{both}").contains("marker"));
}

#[tokio::test]
async fn receive_failure_retains_cleanup_failure_without_formatting_either() {
    let mut supervisor = Supervisor::new(crate::budget().unwrap());
    supervisor
        .register("worker", |startup| async {
            let shutdown = startup.acknowledge_started();
            shutdown.draining().await;
            Ok(shutdown.stopped())
        })
        .unwrap();
    supervisor
        .on_cleanup("pool", || async {
            Err(std::io::Error::other("cleanup-marker").into())
        })
        .unwrap();
    let running = supervisor.start();
    running.status().wait_ready().await.unwrap();
    let (sender, receiver) = oneshot::channel::<()>();
    drop(sender);
    let error = complete(running, async {
        receiver.await?;
        Ok(())
    })
    .await
    .unwrap_err();
    let both = error.downcast_ref::<WorkAndShutdownFailure>().unwrap();
    assert!(both.work.is::<oneshot::error::RecvError>());
    let ShutdownFailure::Report(report) = &both.shutdown else {
        panic!("expected the retained cleanup report")
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
    assert!(!format!("{both:?}").contains("cleanup-marker"));
    assert!(!format!("{both}").contains("cleanup-marker"));
}

#[tokio::test]
async fn uncertain_settlement_keeps_dependency_cleanup_skipped() {
    let mut supervisor = Supervisor::new(crate::budget().unwrap());
    supervisor
        .register("worker", |startup| async {
            let shutdown = startup.acknowledge_started();
            shutdown.draining().await;
            panic!("worker-panic-marker");
            #[allow(unreachable_code)]
            Ok(shutdown.stopped())
        })
        .unwrap();
    let closed = Arc::new(AtomicBool::new(false));
    let cleanup_closed = closed.clone();
    supervisor
        .on_cleanup("pool", move || async move {
            cleanup_closed.store(true, Ordering::SeqCst);
            Ok(())
        })
        .unwrap();
    let running = supervisor.start();
    running.status().wait_ready().await.unwrap();
    let error = complete(running, async { Ok(()) }).await.unwrap_err();
    let ShutdownFailure::Report(report) = error.downcast_ref::<ShutdownFailure>().unwrap() else {
        panic!("expected the retained panic report")
    };
    assert!(!closed.load(Ordering::SeqCst));
    assert_eq!(report.cleanup.skipped.len(), 1);
    assert!(!format!("{error:?}").contains("worker-panic-marker"));
}
