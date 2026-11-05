use super::*;
use batter_core::{
    command::CommandCause,
    operation::{Interruption, OperationOwner, PhasedFailure, PhasedOperation},
};
use std::convert::Infallible;

fn parent() -> OperationOwner {
    OperationOwner::new(Duration::from_secs(10)).expect("valid operation budget")
}

#[tokio::test(flavor = "current_thread")]
async fn completed_phases_preserve_the_submitters_dispatch_and_span() {
    let ambient = Logs::new();
    let _ambient = tracing::dispatcher::set_default(&ambient.dispatch);
    let driver = Logs::new();
    let parent = parent();
    let running = driver.within("phased.normal", || {
        PhasedOperation::new(
            parent.context(),
            Duration::from_secs(1),
            async |_work| {
                let _guard = DropMarker {
                    name: "phased.normal.work",
                    dropped: None,
                };
                tokio::task::yield_now().await;
                Ok::<_, Infallible>(42)
            },
            async |scope| {
                let _guard = DropMarker {
                    name: "phased.normal.finalization",
                    dropped: None,
                };
                assert_eq!(scope.work().result().ok(), Some(&42));
                tokio::task::yield_now().await;
                tracing::info!("phased.normal.recorded");
                Ok::<_, Infallible>(())
            },
        )
        .unwrap()
        .start()
    });
    assert!(running.wait().await.unwrap().is_success());
    for event in [
        "phased.normal.work",
        "phased.normal.finalization",
        "phased.normal.recorded",
    ] {
        driver.one_event(event, "phased.normal");
        assert!(!ambient.text().contains(event), "{}", ambient.text());
    }
}

#[tokio::test(flavor = "current_thread")]
async fn owner_loss_preserves_dispatch_through_work_and_finalization_destruction() {
    let ambient = Logs::new();
    let _ambient = tracing::dispatcher::set_default(&ambient.dispatch);
    let driver = Logs::new();
    let parent = parent();
    let (started, ready) = oneshot::channel();
    let running = driver.within("phased.owner", || {
        PhasedOperation::new(
            parent.context(),
            Duration::from_secs(1),
            async move |_work| {
                let _guard = DropMarker {
                    name: "phased.owner.work",
                    dropped: None,
                };
                started.send(()).unwrap();
                pending::<Result<(), Infallible>>().await
            },
            async |scope| {
                let _guard = DropMarker {
                    name: "phased.owner.finalization",
                    dropped: None,
                };
                assert!(matches!(
                    scope.work().result(),
                    Err(CommandCause::Interrupted(Interruption::Cancelled))
                ));
                tokio::task::yield_now().await;
                tracing::info!("phased.owner.recorded");
                Ok::<_, Infallible>(())
            },
        )
        .unwrap()
        .start()
    });
    ready.await.unwrap();
    let observer = running.observer();
    drop(running);
    let PhasedFailure::Report(report) = observer.wait().await.unwrap_err() else {
        panic!("work cancellation must produce a complete report")
    };
    assert!(report.finalization().is_success());
    for event in [
        "phased.owner.work",
        "phased.owner.finalization",
        "phased.owner.recorded",
    ] {
        driver.one_event(event, "phased.owner");
        assert!(!ambient.text().contains(event), "{}", ambient.text());
    }
}

#[tokio::test(flavor = "current_thread")]
async fn skipped_work_factory_is_destroyed_under_its_submitters_dispatch() {
    let ambient = Logs::new();
    let _ambient = tracing::dispatcher::set_default(&ambient.dispatch);
    let driver = Logs::new();
    let parent = parent();
    let capture = DropMarker {
        name: "phased.skipped.capture",
        dropped: None,
    };
    let running = driver.within("phased.skipped", || {
        PhasedOperation::new(
            parent.context(),
            Duration::from_secs(1),
            async move |_work| {
                let _capture = capture;
                panic!("the cancelled work factory must not run");
                #[expect(unreachable_code, reason = "names the uncalled phase output")]
                Ok::<_, Infallible>(())
            },
            async |_scope| Ok::<_, Infallible>(()),
        )
        .unwrap()
        .start()
    });
    let observer = running.observer();
    // No yield: cancellation precedes the coordinator's first poll.
    drop(running);
    let PhasedFailure::Report(report) = observer.wait().await.unwrap_err() else {
        panic!("expected work cancellation")
    };
    assert!(matches!(
        report.work().result(),
        Err(CommandCause::Interrupted(Interruption::Cancelled))
    ));
    assert!(report.finalization().is_success());
    driver.one_event("phased.skipped.capture", "phased.skipped");
    assert!(!ambient.text().contains("phased.skipped.capture"));
}

#[tokio::test(flavor = "current_thread")]
async fn unclaimed_phase_results_are_destroyed_under_the_publishers_dispatch() {
    let ambient = Logs::new();
    let _ambient = tracing::dispatcher::set_default(&ambient.dispatch);
    let driver = Logs::new();
    let parent = parent();
    let (dropped, ended) = oneshot::channel();
    let (entered, ready) = oneshot::channel();
    let (release, released) = oneshot::channel();
    let value = DropMarker {
        name: "phased.unclaimed.work",
        dropped: Some(dropped),
    };
    let running = driver.within("phased.unclaimed", || {
        PhasedOperation::new(
            parent.context(),
            Duration::from_secs(1),
            async move |_work| Ok::<_, Infallible>(value),
            async move |_scope| {
                entered.send(()).unwrap();
                released.await.unwrap();
                Ok::<_, Infallible>(DropMarker {
                    name: "phased.unclaimed.finalization",
                    dropped: None,
                })
            },
        )
        .unwrap()
        .start()
    });
    ready.await.unwrap();
    drop(running);
    release.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(10), ended)
        .await
        .expect("unclaimed report destroyed")
        .unwrap();
    for event in ["phased.unclaimed.work", "phased.unclaimed.finalization"] {
        driver.one_event(event, "phased.unclaimed");
        assert!(!ambient.text().contains(event), "{}", ambient.text());
    }
}
