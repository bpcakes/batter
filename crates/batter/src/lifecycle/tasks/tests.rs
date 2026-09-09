use super::*;
use crate::lifecycle::{ProcessAdmissionError, ProcessHandle};
use std::{convert::Infallible, future::poll_fn, io, task::Poll};
use tokio::sync::oneshot;

#[tokio::test]
async fn cancelled_join_wait_keeps_each_failure_owned_and_recorded_once() {
    for outcome in [
        TaskOutcome::UnexpectedExit,
        TaskOutcome::Failed,
        TaskOutcome::Panicked,
    ] {
        let handle = ShutdownHandle::new();
        handle.shared.start_driver();
        handle.mark_ready();
        let (process, _queued) = ProcessHandle::new(handle.clone(), 1).unwrap();
        let (release, released) = oneshot::channel();
        let mut tasks = TaskSet::default();
        tasks.spawn_component(
            Component {
                name: "component",
                factory: Box::new(move |_| {
                    Box::pin(async move {
                        released.await.unwrap();
                        match outcome {
                            TaskOutcome::UnexpectedExit => Ok(()),
                            TaskOutcome::Failed => Err(io::Error::other("retained failure").into()),
                            TaskOutcome::Panicked => panic!("component panic"),
                            _ => unreachable!(),
                        }
                    })
                }),
            },
            &handle,
        );
        {
            let wait = tasks.next_exit(&handle);
            tokio::pin!(wait);
            poll_fn(|cx| {
                assert!(wait.as_mut().poll(cx).is_pending());
                Poll::Ready(())
            })
            .await;
            // Destroy this pending waiter without destroying the owned task.
        }
        release.send(()).unwrap();
        assert_eq!(
            tasks.next_exit(&handle).await,
            Some(ShutdownCause::ComponentExit("component"))
        );
        // Failure closes finite admission before the cause reaches the caller.
        assert!(matches!(
            process.try_spawn("later", |_| async { Ok::<_, Infallible>(()) }),
            Err(ProcessAdmissionError::Closed)
        ));
        tasks.collect_ready(&handle);
        assert!(tasks.is_empty());
        let summary = tasks.finish();
        assert_eq!(summary.records.len(), 1);
        let record = &summary.records[0];
        assert_eq!(record.name, "component");
        assert_eq!(record.outcome, outcome);
        match outcome {
            TaskOutcome::UnexpectedExit => assert!(record.error.is_none()),
            TaskOutcome::Failed => assert_eq!(
                record
                    .error
                    .as_ref()
                    .unwrap()
                    .downcast_ref::<io::Error>()
                    .unwrap()
                    .to_string(),
                "retained failure"
            ),
            TaskOutcome::Panicked => assert!(
                record
                    .error
                    .as_ref()
                    .unwrap()
                    .downcast_ref::<JoinError>()
                    .unwrap()
                    .is_panic()
            ),
            _ => unreachable!(),
        }
        assert_eq!(summary.unsafe_exit, outcome == TaskOutcome::Panicked);
        assert_eq!(summary.completed, 0);
        assert!(summary.unjoined.is_empty());
    }
}

#[tokio::test]
async fn finishing_unjoined_tasks_releases_ownership_without_claiming_termination() {
    let (started, ready) = oneshot::channel();
    let (dropped, ended) = oneshot::channel();
    struct Capture(Option<oneshot::Sender<()>>);
    impl Drop for Capture {
        fn drop(&mut self) {
            let _ = self.0.take().unwrap().send(());
        }
    }
    let mut tasks = TaskSet::default();
    tasks.spawn_process(process::QueuedProcess {
        name: "unfinished",
        future: Box::pin(async move {
            let _capture = Capture(Some(dropped));
            started.send(()).unwrap();
            std::future::pending().await
        }),
    });
    ready.await.unwrap();
    let summary = tasks.finish();
    assert_eq!(summary.unjoined, ["unfinished"]);
    assert!(summary.unsafe_exit);
    assert!(summary.records.is_empty());
    // Independently observe this yielding fixture's destruction after finish.
    tokio::time::timeout(std::time::Duration::from_secs(1), ended)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(summary.unjoined, ["unfinished"]);
}
