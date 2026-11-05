use crate::operation::{OperationOwner, PhasedOperation};
use std::{
    future::{Future, poll_fn},
    io::{self, Write},
    sync::{Arc, Mutex},
    task::Poll,
    time::Duration,
};

#[derive(Clone, Default)]
struct Logs(Arc<Mutex<Vec<u8>>>);

impl Write for Logs {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Logs {
    fn dispatch(&self) -> tracing::Dispatch {
        let writer = self.clone();
        crate::test_dispatch::new(
            tracing_subscriber::fmt()
                .with_writer(move || writer.clone())
                .with_ansi(false)
                .without_time()
                .finish(),
        )
    }
    fn drops(&self) -> Vec<String> {
        String::from_utf8(self.0.lock().unwrap().clone())
            .unwrap()
            .lines()
            .filter(|line| line.contains("phase evidence dropped"))
            .map(str::to_owned)
            .collect()
    }
}

// Deliberately not Clone: claims may only share evidence, never copy it.
struct Evidence;
impl Drop for Evidence {
    fn drop(&mut self) {
        tracing::info!("phase evidence dropped");
    }
}

#[derive(Clone, Copy)]
enum Finalization {
    Success,
    Failure,
}

#[derive(Clone, Copy)]
enum Observation {
    Unread,
    ResultDroppedFirst,
    ResultDroppedLast,
}

async fn publication_case(finalization: Finalization, observation: Observation) {
    let producer = Logs::default();
    let ambient = Logs::default();
    let parent = OperationOwner::new(Duration::from_secs(30)).unwrap();
    let (release, gate) = tokio::sync::oneshot::channel();
    let running = tracing::dispatcher::with_default(&producer.dispatch(), || {
        let parent_span = tracing::info_span!("publication.parent");
        let child = tracing::info_span!(parent: &parent_span, "publication.producer");
        let _entered = child.enter();
        PhasedOperation::new(
            parent.context(),
            Duration::from_secs(1),
            async move |_| {
                gate.await.unwrap();
                Ok::<_, ()>(Evidence)
            },
            async move |_| match finalization {
                Finalization::Failure => Err(Evidence),
                Finalization::Success => Ok(Evidence),
            },
        )
        .unwrap()
        .start()
    });
    let observer = running.observer();
    let duplicate = observer.clone();
    // Neither an unpolled nor a polled-and-cancelled waiter claims evidence.
    drop(duplicate.wait());
    let mut waiting = Box::pin(observer.wait());
    poll_fn(|cx| {
        assert!(waiting.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    drop(waiting);
    release.send(()).unwrap();
    // Sender closure proves publication finished. Do not call wait(), inspect
    // the stored result or rely on producer span closure to reach this point.
    let mut stored = observer.completion.clone();
    while stored.changed().await.is_ok() {}
    assert!(producer.drops().is_empty());
    assert!(ambient.drops().is_empty());
    let claim = !matches!(observation, Observation::Unread);
    let result = if claim {
        Some(observer.wait().await)
    } else {
        None
    };
    tracing::dispatcher::with_default(&ambient.dispatch(), || {
        let caller = tracing::info_span!("publication.caller");
        let _entered = caller.enter();
        let returned = result.clone();
        drop(result);
        // Exercise either last ownership location after the actual claim.
        let returned = if matches!(observation, Observation::ResultDroppedLast) {
            returned
        } else {
            drop(returned);
            None
        };
        drop(running);
        drop(observer);
        drop(duplicate);
        drop(stored);
        drop(returned);
    });
    let (expected, absent, span) = if claim {
        (&ambient, &producer, "publication.caller")
    } else {
        (&producer, &ambient, "publication.producer")
    };
    let drops = expected.drops();
    assert_eq!(drops.len(), 2, "each retained value/error destroyed once");
    assert!(drops.iter().all(|line| line.contains(span)), "{drops:?}");
    assert!(absent.drops().is_empty());
}

#[tokio::test]
async fn unread_success_and_failure_remain_guarded_after_publication_ends() {
    for finalization in [Finalization::Success, Finalization::Failure] {
        publication_case(finalization, Observation::Unread).await;
    }
}

#[tokio::test]
async fn actual_claim_transfers_success_and_failure_destruction_in_either_drop_order() {
    for finalization in [Finalization::Success, Finalization::Failure] {
        for observation in [
            Observation::ResultDroppedFirst,
            Observation::ResultDroppedLast,
        ] {
            publication_case(finalization, observation).await;
        }
    }
}
