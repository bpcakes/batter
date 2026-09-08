use super::support::{Case, poll_pending, yields};
use batter::{
    admission::{Admission, AdmissionError, Bulkhead},
    operation::{Interruption, OperationContext, OperationError},
};
use std::{
    convert::Infallible,
    future::pending,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::sync::{Barrier, oneshot};

pub async fn hierarchy_and_drop(case: Case) {
    let parent = OperationContext::new(Duration::from_secs(30)).unwrap();
    let child = parent.child(Duration::from_secs(60)).unwrap();
    let sibling = parent.child(Duration::from_secs(60)).unwrap();
    assert_eq!(child.deadline(), parent.deadline());
    child.cancel();
    assert_eq!(child.check(), Err(Interruption::Cancelled));
    assert!(parent.check().is_ok() && sibling.check().is_ok());
    let (sender, receiver) = oneshot::channel();
    let mut run = Box::pin(sibling.run("dropped-scope", |scope| async move {
        sender.send(scope).unwrap();
        pending::<Result<(), Infallible>>().await
    }));
    poll_pending(run.as_mut()).await;
    let escaped = receiver.await.unwrap();
    drop(run);
    assert_eq!(escaped.check(), Err(Interruption::Cancelled));
    assert!(parent.check().is_ok() && sibling.check().is_ok());
    let invoked = Arc::new(AtomicBool::new(false));
    let flag = invoked.clone();
    drop(sibling.run("unpolled", |_| {
        flag.store(true, Ordering::SeqCst);
        async { Ok::<_, Infallible>(()) }
    }));
    assert!(!invoked.load(Ordering::SeqCst));
    parent.cancel();
    assert_eq!(sibling.check(), Err(Interruption::Cancelled));
    case.event("downward-cancellation-and-drop-checked");
}

pub async fn branch_priority(case: Case, cancel: bool, expire: bool) {
    let parent = OperationContext::new(Duration::from_secs(30)).unwrap();
    let duration = if expire {
        Duration::from_millis(20)
    } else {
        Duration::from_secs(20)
    };
    let context = parent.child(duration).unwrap();
    let (scope_tx, scope_rx) = oneshot::channel();
    let (finish, finished) = oneshot::channel();
    let mut run = Box::pin(context.run("simultaneous-branches", |scope| async move {
        scope_tx.send(scope).unwrap();
        finished.await.unwrap();
        Ok::<_, Infallible>(37)
    }));
    poll_pending(run.as_mut()).await;
    let escaped = scope_rx.await.unwrap();
    if expire {
        tokio::time::sleep_until(context.deadline()).await;
    }
    if cancel {
        context.cancel();
    }
    finish.send(()).unwrap();
    case.event("branches-ready-before-repoll");
    let result = run.await;
    match (cancel, expire) {
        (true, _) => assert!(matches!(
            result,
            Err(OperationError::Interrupted(Interruption::Cancelled))
        )),
        (false, true) => assert!(matches!(
            result,
            Err(OperationError::Interrupted(Interruption::DeadlineExceeded))
        )),
        (false, false) => assert_eq!(result.unwrap(), 37),
    }
    assert_eq!(escaped.check(), Err(Interruption::Cancelled));
    assert!(parent.check().is_ok());
}

pub async fn completion_race(case: Case, delay: u64) {
    let parent = OperationContext::new(Duration::from_secs(30)).unwrap();
    let context = parent.child(Duration::from_secs(20)).unwrap();
    let sibling = parent.child(Duration::from_secs(20)).unwrap();
    let run_context = context.clone();
    let barrier = Arc::new(Barrier::new(3));
    let completing = barrier.clone();
    let (scope_tx, scope_rx) = oneshot::channel();
    let run = tokio::spawn(async move {
        run_context
            .run("completion-cancellation", |scope| async move {
                scope_tx.send(scope).unwrap();
                completing.wait().await;
                yields(delay).await;
                case.event("operation-returning");
                Ok::<_, Infallible>(71)
            })
            .await
    });
    let escaped = scope_rx.await.unwrap();
    let cancelling = barrier.clone();
    let cancel = tokio::spawn(async move {
        cancelling.wait().await;
        yields(delay.rotate_left(3)).await;
        context.cancel();
        case.event("operation-cancel-returned");
    });
    barrier.wait().await;
    cancel.await.unwrap();
    assert!(matches!(
        run.await.unwrap(),
        Ok(71) | Err(OperationError::Interrupted(Interruption::Cancelled))
    ));
    assert_eq!(escaped.check(), Err(Interruption::Cancelled));
    assert!(parent.check().is_ok() && sibling.check().is_ok());
}

pub async fn bulkhead_preflight(case: Case, delay: u64) {
    let bulkhead = Bulkhead::new(1).unwrap();
    let fresh = OperationContext::new(Duration::from_secs(30)).unwrap();
    let permit = bulkhead.enter(&fresh, Admission::Reject).await.unwrap();
    let context = fresh.child(Duration::from_secs(20)).unwrap();
    let waiter_context = context.clone();
    let waiting_bulkhead = bulkhead.clone();
    let waiter = tokio::spawn(async move {
        waiting_bulkhead
            .enter(&waiter_context, Admission::Wait)
            .await
    });
    let release = tokio::spawn(async move {
        yields(delay).await;
        drop(permit);
    });
    yields(delay.rotate_left(7)).await;
    context.cancel();
    release.await.unwrap();
    match waiter.await.unwrap() {
        Ok(permit) => drop(permit),
        Err(AdmissionError::Interrupted(Interruption::Cancelled)) => {}
        Err(_) => panic!("unexpected bulkhead cancellation race outcome"),
    }
    // Preflight/acquisition is not atomic: work still uses its operation boundary.
    let result = context
        .run("after-admission", |_| async { Ok::<_, Infallible>(()) })
        .await;
    assert!(matches!(
        result,
        Err(OperationError::Interrupted(Interruption::Cancelled))
    ));
    assert!(bulkhead.enter(&fresh, Admission::Reject).await.is_ok());
    assert!(fresh.check().is_ok());
    case.event("bulkhead-race-no-leak");
}
