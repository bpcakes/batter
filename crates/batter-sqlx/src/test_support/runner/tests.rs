use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tokio::sync::oneshot;

fn run<T, E>(driver: JoinHandle<FixtureReport<T, E>>) -> FixtureRun<T, E> {
    FixtureRun {
        state: RunState::Running(driver),
        progress: Default::default(),
    }
}

fn report<T>(value: T) -> FixtureReport<T, ()> {
    FixtureReport {
        body: Ok(value),
        acquisitions: Vec::new(),
        databases: Vec::new(),
        drain: Ok(()),
    }
}

struct Payload {
    value: u32,
    drops: Arc<AtomicUsize>,
}

impl Drop for Payload {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}

#[tokio::test(start_paused = true)]
async fn pending_observation_resumes_and_repeated_waits_retain_one_report() {
    let drops = Arc::new(AtomicUsize::new(0));
    let payload = Payload {
        value: 42,
        drops: drops.clone(),
    };
    let (release, released) = oneshot::channel();
    let mut run = run(tokio::spawn(async move {
        released.await.unwrap();
        report(payload)
    }));
    assert!(
        run.wait_for(Duration::from_millis(1))
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    release.send(()).unwrap();
    let address = {
        let observed = run.wait().await.unwrap();
        assert_eq!(observed.body.as_ref().unwrap().value, 42);
        std::ptr::from_ref(&*observed)
    };
    {
        let observed = run
            .wait_for(Duration::from_millis(1))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(std::ptr::from_ref(&*observed), address);
    }
    let owned = run.into_report().await.unwrap();
    assert_eq!(owned.body.as_ref().unwrap().value, 42);
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    drop(owned);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn driver_failure_is_cached_for_observation_and_consuming_extraction() {
    let mut run: FixtureRun<(), ()> = run(tokio::spawn(async { panic!("fixture driver failure") }));
    let (id, address) = {
        let error = run.wait().await.unwrap_err();
        assert!(error.is_panic());
        (error.id(), std::ptr::from_ref(error))
    };
    {
        let error = run.wait_for(Duration::from_secs(1)).await.unwrap_err();
        assert_eq!(error.id(), id);
        assert_eq!(std::ptr::from_ref(error), address);
    }
    let error = run.into_report().await.unwrap_err();
    assert_eq!(error.id(), id);
    assert_eq!(
        *error.into_panic().downcast::<&str>().unwrap(),
        "fixture driver failure"
    );
}

#[tokio::test]
async fn direct_consuming_wait_preserves_body_and_cleanup_failures() {
    let run = run(tokio::spawn(async {
        FixtureReport::<(), _> {
            body: Err(BodyFailure::Returned("body failure")),
            acquisitions: Vec::new(),
            databases: Vec::new(),
            drain: Err(FixtureError::BatchCapacity),
        }
    }));
    let report = run.into_report().await.unwrap();
    assert!(!report.is_ok());
    assert!(matches!(
        report.body,
        Err(BodyFailure::Returned("body failure"))
    ));
    assert!(matches!(report.drain, Err(FixtureError::BatchCapacity)));
}

#[tokio::test]
async fn losing_the_waiter_or_unpolled_consuming_future_does_not_abort_the_driver() {
    for consume in [false, true] {
        let (release, released) = oneshot::channel();
        let (finished, finish) = oneshot::channel();
        let run = run(tokio::spawn(async move {
            released.await.unwrap();
            finished.send(()).unwrap();
            report(())
        }));
        if consume {
            drop(run.into_report());
        } else {
            drop(run);
        }
        release.send(()).unwrap();
        finish.await.unwrap();
    }
}
