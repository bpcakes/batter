use batter::{
    ConfigurationError,
    operation::{Interruption, OperationContext, OperationError},
};
use std::{
    future::pending,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::time::Instant;

struct Dropped(Arc<AtomicBool>);
impl Drop for Dropped {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

#[test]
fn zero_budget_is_invalid() {
    assert!(OperationContext::new(Duration::ZERO).is_err());
    assert!(OperationContext::new(Duration::MAX).is_err());
}

#[tokio::test(start_paused = true)]
async fn child_cannot_extend_deadline() {
    let parent = OperationContext::new(Duration::from_secs(1)).unwrap();
    let child = parent.child(Duration::from_secs(20)).unwrap();
    assert_eq!(parent.deadline(), child.deadline());
    child.cancel();
    assert_eq!(child.check(), Err(Interruption::Cancelled));
    assert!(parent.check().is_ok());
}

#[tokio::test(start_paused = true)]
async fn parent_cancellation_propagates_to_child() {
    let parent = OperationContext::new(Duration::from_secs(10)).unwrap();
    let child = parent.child(Duration::from_secs(1)).unwrap();
    parent.cancel();
    assert_eq!(child.check(), Err(Interruption::Cancelled));
}

#[tokio::test(start_paused = true)]
async fn expired_context_does_not_invoke_factory() {
    let context = OperationContext::at(Instant::now());
    let result = context
        .run("expired", |_| {
            panic!("factory must not be invoked");
            #[allow(unreachable_code)]
            async {
                Ok::<_, std::io::Error>(())
            }
        })
        .await;
    assert!(matches!(
        result,
        Err(OperationError::Interrupted(Interruption::DeadlineExceeded))
    ));
}

#[tokio::test(start_paused = true)]
async fn cancellation_wins_when_deadline_is_also_expired() {
    let context = OperationContext::at(Instant::now());
    context.cancel();
    let result = context
        .run("cancelled", |_| async { Ok::<_, std::io::Error>(()) })
        .await;
    assert!(matches!(
        result,
        Err(OperationError::Interrupted(Interruption::Cancelled))
    ));
}

#[tokio::test(start_paused = true)]
async fn success_cancels_scope_but_not_parent() {
    let context = OperationContext::new(Duration::from_secs(1)).unwrap();
    let escaped = context
        .run(
            "success",
            |scope| async move { Ok::<_, std::io::Error>(scope) },
        )
        .await
        .unwrap();
    assert_eq!(escaped.check(), Err(Interruption::Cancelled));
    assert!(context.check().is_ok());
}

#[tokio::test(start_paused = true)]
async fn original_application_error_survives() {
    let context = OperationContext::new(Duration::from_secs(1)).unwrap();
    let result: Result<(), _> = context
        .run("failure", |_| async {
            Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "original",
            ))
        })
        .await;
    match result {
        Err(OperationError::Failed(error)) => {
            assert_eq!(error.kind(), std::io::ErrorKind::NotFound)
        }
        other => panic!("unexpected: {other:?}"),
    }
}

#[tokio::test(start_paused = true)]
async fn timeout_drops_the_owned_future() {
    let dropped = Arc::new(AtomicBool::new(false));
    let flag = dropped.clone();
    let context = OperationContext::new(Duration::from_secs(1)).unwrap();
    let result = context
        .run("timeout", |_| async move {
            let _guard = Dropped(flag);
            pending::<Result<(), std::io::Error>>().await
        })
        .await;
    assert!(matches!(
        result,
        Err(OperationError::Interrupted(Interruption::DeadlineExceeded))
    ));
    assert!(dropped.load(Ordering::SeqCst));
}

#[tokio::test]
async fn dropping_run_notifies_its_child_scope() {
    let context = OperationContext::new(Duration::from_secs(60)).unwrap();
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        context
            .run("dropped", |scope| async move {
                sender.send(scope).unwrap();
                pending::<Result<(), std::io::Error>>().await
            })
            .await
    });
    let scope = receiver.await.unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert_eq!(scope.check(), Err(Interruption::Cancelled));
}

#[tokio::test(start_paused = true)]
async fn borrowed_application_futures_are_supported() {
    let mut values = Vec::new();
    let context = OperationContext::new(Duration::from_secs(1)).unwrap();
    context
        .run("borrowed", |_| async {
            values.push(42);
            Ok::<_, std::io::Error>(())
        })
        .await
        .unwrap();
    assert_eq!(values, vec![42]);
}

#[tokio::test]
async fn panic_is_not_converted_to_expected_failure() {
    let task = tokio::spawn(async {
        let context = OperationContext::new(Duration::from_secs(1)).unwrap();
        context
            .run("panic", |_| async {
                panic!("programming defect");
                #[allow(unreachable_code)]
                Ok::<_, std::io::Error>(())
            })
            .await
    });
    assert!(task.await.unwrap_err().is_panic());
}

#[tokio::test(start_paused = true)]
async fn work_exhaustion_leaves_time_to_await_real_finalization() {
    let parent = OperationContext::new(Duration::from_secs(10)).unwrap();
    let phases = parent.reserve_finalization(Duration::from_secs(3)).unwrap();
    let deadline = parent.deadline();
    assert_eq!(phases.work().deadline(), deadline - Duration::from_secs(3));
    assert_eq!(phases.finalization().deadline(), deadline);
    assert_eq!(
        phases
            .work()
            .child(Duration::from_secs(30))
            .unwrap()
            .deadline(),
        phases.work().deadline()
    );

    let (sender, receiver) = tokio::sync::oneshot::channel();
    let work = phases
        .work()
        .run("reserved.work", |scope| async move {
            sender.send(scope).unwrap();
            tokio::time::sleep(Duration::from_secs(8)).await;
            Ok::<_, std::io::Error>(())
        })
        .await;
    assert!(matches!(
        work,
        Err(OperationError::Interrupted(Interruption::DeadlineExceeded))
    ));
    assert_eq!(
        receiver.await.unwrap().check(),
        Err(Interruption::Cancelled)
    );
    assert!(phases.finalization().remaining() >= Duration::from_millis(2990));

    let finalized = Arc::new(AtomicBool::new(false));
    let flag = finalized.clone();
    phases
        .finalization()
        .run("reserved.finalization", |_| async move {
            tokio::time::sleep(Duration::from_secs(2)).await;
            flag.store(true, Ordering::SeqCst);
            Ok::<_, std::io::Error>(())
        })
        .await
        .unwrap();
    assert!(finalized.load(Ordering::SeqCst));
    assert_eq!(parent.deadline(), deadline);
    assert!(parent.check().is_ok());
}

#[tokio::test(start_paused = true)]
async fn finalization_uses_original_deadline_and_cannot_reset_its_allowance() {
    let parent = OperationContext::new(Duration::from_secs(10)).unwrap();
    let phases = parent.reserve_finalization(Duration::from_secs(3)).unwrap();
    tokio::time::advance(Duration::from_secs(8)).await;
    assert_eq!(phases.finalization().remaining(), Duration::from_secs(2));
    let result = phases
        .finalization()
        .run("bounded.finalization", |_| async {
            tokio::time::sleep(Duration::from_secs(3)).await;
            Ok::<_, std::io::Error>(())
        })
        .await;
    assert!(matches!(
        result,
        Err(OperationError::Interrupted(Interruption::DeadlineExceeded))
    ));
    assert!(parent.remaining().is_zero());
}

#[tokio::test(start_paused = true)]
async fn work_cancellation_preserves_finalization_but_parent_cancellation_does_not() {
    let parent = OperationContext::new(Duration::from_secs(10)).unwrap();
    let phases = parent.reserve_finalization(Duration::from_secs(3)).unwrap();
    phases.work().cancel();
    assert_eq!(phases.work().check(), Err(Interruption::Cancelled));
    assert!(phases.finalization().check().is_ok());
    assert!(parent.check().is_ok());
    parent.cancel();
    assert_eq!(phases.finalization().check(), Err(Interruption::Cancelled));
}

#[tokio::test(start_paused = true)]
async fn reserve_must_leave_positive_work_time_at_allocation() {
    let parent = OperationContext::new(Duration::from_secs(10)).unwrap();
    assert!(parent.reserve_finalization(Duration::ZERO).is_err());
    assert!(parent.reserve_finalization(Duration::MAX).is_err());
    assert!(matches!(
        parent.reserve_finalization(Duration::from_secs(10)),
        Err(ConfigurationError::InvalidReserve)
    ));
    tokio::time::advance(Duration::from_secs(8)).await;
    assert!(matches!(
        parent.reserve_finalization(Duration::from_secs(3)),
        Err(ConfigurationError::InvalidReserve)
    ));
    tokio::time::advance(Duration::from_secs(2)).await;
    assert!(matches!(
        parent.reserve_finalization(Duration::from_nanos(1)),
        Err(ConfigurationError::InvalidReserve)
    ));
}
