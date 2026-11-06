//! Consumer-facing ownership and refusal contracts for async bulkhead execution.
use batter_core::{
    admission::{Admission, AdmissionError, Bulkhead, BulkheadCapacity, BulkheadError},
    operation::{Interruption, OperationContext, OperationError, OperationOwner},
};
use std::{
    future::{Future, pending},
    pin::pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Waker},
    time::Duration,
};

fn bulkhead() -> Bulkhead {
    Bulkhead::new(BulkheadCapacity::new(1).expect("positive capacity"))
}
fn owner() -> OperationOwner {
    OperationOwner::new(Duration::from_secs(1)).expect("positive budget")
}
fn poll_pending<F: Future>(future: std::pin::Pin<&mut F>) {
    assert!(
        future
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending(),
        "work must still be pending"
    );
}

struct OnDrop(Box<dyn FnOnce() + Send>);
impl Drop for OnDrop {
    fn drop(&mut self) {
        std::mem::replace(&mut self.0, Box::new(|| {}))();
    }
}

#[tokio::test]
async fn capacity_one_serializes_factories_and_releases_after_future_destruction() {
    let bulkhead = bulkhead();
    let owner = owner();
    let events = Arc::new(Mutex::new(Vec::new()));
    let signal = Arc::new(tokio::sync::Notify::new());
    let events1 = events.clone();
    let signal1 = signal.clone();
    let mut first = Box::pin(bulkhead.run(
        owner.context(),
        Admission::Reject,
        "first",
        move |_| async move {
            events1.lock().unwrap().push("first start");
            let _drop = OnDrop(Box::new(move || events1.lock().unwrap().push("first drop")));
            signal1.notified().await;
            Ok::<_, ()>(())
        },
    ));
    poll_pending(first.as_mut());
    let events2 = events.clone();
    let mut second = Box::pin(bulkhead.run(
        owner.context(),
        Admission::Wait,
        "second",
        move |_| async move {
            events2.lock().unwrap().push("second start");
            Ok::<_, ()>(())
        },
    ));
    poll_pending(second.as_mut());
    assert_eq!(*events.lock().unwrap(), ["first start"]);
    signal.notify_one();
    first.await.unwrap();
    second.await.unwrap();
    assert_eq!(
        *events.lock().unwrap(),
        ["first start", "first drop", "second start"]
    );
}

#[tokio::test]
async fn drop_cancels_child_and_destroys_work_before_releasing_permit() {
    let bulkhead = bulkhead();
    let owner = owner();
    let checked = Arc::new(AtomicBool::new(false));
    let child = Arc::new(Mutex::new(None::<OperationContext>));
    let mut run = Box::pin(bulkhead.run(owner.context(), Admission::Reject, "drop", {
        let (b, checked, child) = (bulkhead.clone(), checked.clone(), child.clone());
        move |context| async move {
            *child.lock().unwrap() = Some(context.clone());
            let _drop = OnDrop(Box::new(move || {
                let fresh = crate::owner();
                let enter = b.enter(fresh.context(), Admission::Reject);
                let result = pin!(enter).poll(&mut Context::from_waker(Waker::noop()));
                assert!(matches!(
                    result,
                    std::task::Poll::Ready(Err(AdmissionError::Overloaded))
                ));
                checked.store(true, Ordering::SeqCst);
            }));
            pending::<Result<(), ()>>().await
        }
    }));
    poll_pending(run.as_mut());
    drop(run);
    assert!(checked.load(Ordering::SeqCst));
    assert_eq!(
        child.lock().unwrap().as_ref().unwrap().check(),
        Err(Interruption::Cancelled)
    );
    assert!(
        bulkhead
            .enter(owner.context(), Admission::Reject)
            .await
            .is_ok()
    );
}

#[tokio::test(start_paused = true)]
async fn admission_and_execution_share_the_original_deadline() {
    let bulkhead = bulkhead();
    let owner = owner();
    let held = bulkhead
        .enter(owner.context(), Admission::Reject)
        .await
        .unwrap();
    let mut run = Box::pin(bulkhead.run(
        owner.context(),
        Admission::Wait,
        "deadline",
        |scope| async move {
            assert!(scope.remaining() <= Duration::from_millis(400));
            pending::<Result<(), ()>>().await
        },
    ));
    poll_pending(run.as_mut());
    tokio::time::advance(Duration::from_millis(600)).await;
    drop(held);
    poll_pending(run.as_mut());
    assert!(matches!(
        run.await,
        Err(BulkheadError::Operation(OperationError::Interrupted(
            Interruption::DeadlineExceeded
        )))
    ));
}

#[tokio::test(start_paused = true)]
async fn late_admission_never_invokes_factory() {
    let bulkhead = bulkhead();
    let owner = owner();
    let held = bulkhead
        .enter(owner.context(), Admission::Reject)
        .await
        .unwrap();
    let waiter = tokio::spawn(async move {
        bulkhead
            .run(owner.context(), Admission::Wait, "late", |_| {
                panic!("late factory");
                #[expect(unreachable_code, reason = "fix the future's result type")]
                pending::<Result<(), ()>>()
            })
            .await
    });
    tokio::task::yield_now().await;
    drop(held);
    tokio::time::advance(Duration::from_secs(2)).await;
    assert!(matches!(
        waiter.await.unwrap(),
        Err(BulkheadError::NotAdmitted(AdmissionError::Interrupted(
            Interruption::DeadlineExceeded
        )))
    ));
}

#[tokio::test]
async fn unpolled_refused_cancelled_and_closed_calls_do_not_invoke_work() {
    let bulkhead = bulkhead();
    let owner = owner();
    let factory = |_| {
        panic!("refused factory");
        #[expect(unreachable_code, reason = "result type")]
        pending::<Result<(), ()>>()
    };
    drop(bulkhead.run(owner.context(), Admission::Reject, "unpolled", factory));
    let held = bulkhead
        .enter(owner.context(), Admission::Reject)
        .await
        .unwrap();
    assert!(matches!(
        bulkhead
            .run(owner.context(), Admission::Reject, "full", factory)
            .await,
        Err(BulkheadError::NotAdmitted(AdmissionError::Overloaded))
    ));
    owner.cancel();
    assert!(matches!(
        bulkhead
            .run(owner.context(), Admission::Wait, "cancelled", factory)
            .await,
        Err(BulkheadError::NotAdmitted(AdmissionError::Interrupted(
            Interruption::Cancelled
        )))
    ));
    drop(held);
    bulkhead.close();
    assert!(matches!(
        bulkhead
            .run(
                crate::owner().context(),
                Admission::Reject,
                "closed",
                factory
            )
            .await,
        Err(BulkheadError::NotAdmitted(AdmissionError::Closed))
    ));
}

#[tokio::test]
async fn concrete_failure_is_retained_and_redacted_and_panic_releases_capacity() {
    let bulkhead = bulkhead();
    let owner = owner();
    let error = bulkhead
        .run(owner.context(), Admission::Reject, "failure", |_| async {
            Err::<(), _>("secret")
        })
        .await
        .unwrap_err();
    assert!(!format!("{error:?} {error}").contains("secret"));
    assert!(matches!(
        error,
        BulkheadError::Operation(OperationError::Failed("secret"))
    ));
    let b = bulkhead.clone();
    let c = owner.context().clone();
    let task = tokio::spawn(async move {
        b.run(&c, Admission::Reject, "panic", |_| async {
            panic!("work panic");
            #[expect(unreachable_code, reason = "result type")]
            Ok::<(), ()>(())
        })
        .await
    });
    assert!(task.await.unwrap_err().is_panic());
    assert!(
        bulkhead
            .enter(owner.context(), Admission::Reject)
            .await
            .is_ok()
    );
}
