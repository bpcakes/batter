//! Invocation evidence and absolute child deadlines share the normal boundary.
use batter_core::operation::{Interruption, OperationOwner, TrackedOperationError};
use std::{
    cell::Cell,
    future::{Future, pending},
    rc::Rc,
    time::Duration,
};
use tokio::time::Instant;

#[tokio::test(start_paused = true)]
async fn refusal_never_invokes_factory_and_preserves_precedence() {
    for (cancelled, expired, expected) in [
        (true, false, Interruption::Cancelled),
        (false, true, Interruption::DeadlineExceeded),
        (true, true, Interruption::Cancelled),
    ] {
        let owner = OperationOwner::new(Duration::from_secs(1)).unwrap();
        if cancelled {
            owner.cancel();
        }
        if expired {
            tokio::time::advance(Duration::from_secs(1)).await;
        }
        let error = owner
            .context()
            .run_tracked("refused", |_| {
                panic!("refused factory invoked");
                #[expect(unreachable_code, reason = "specifies the future type")]
                pending::<Result<(), ()>>()
            })
            .await
            .unwrap_err();
        match error {
            TrackedOperationError::NotStarted(proof) => assert_eq!(proof.reason(), expected),
            other => panic!("wrong phase: {other:?}"),
        }
    }
}

#[tokio::test(start_paused = true)]
async fn invocation_and_interruption_are_distinct_even_before_future_progress() {
    for cancelled in [false, true] {
        let owner = OperationOwner::new(Duration::from_secs(1)).unwrap();
        let invoked = Cell::new(0);
        let result = owner
            .context()
            .run_tracked("started", |_| {
                invoked.set(invoked.get() + 1);
                if cancelled {
                    owner.cancel();
                }
                pending::<Result<(), ()>>()
            })
            .await;
        assert_eq!(invoked.get(), 1);
        assert!(
            matches!(result, Err(TrackedOperationError::Interrupted(reason)) if reason == if cancelled {
                Interruption::Cancelled
            } else {
                Interruption::DeadlineExceeded
            })
        );
    }
}

#[tokio::test(start_paused = true)]
async fn cancellation_and_deadline_win_over_a_ready_completion_after_start() {
    for cancel in [false, true] {
        let owner = OperationOwner::new(Duration::from_secs(1)).unwrap();
        let ready = Cell::new(false);
        let polls = Cell::new(0);
        let run = owner.context().run_tracked("race", |_| {
            std::future::poll_fn(|_| {
                polls.set(polls.get() + 1);
                if ready.get() {
                    std::task::Poll::Ready(Ok::<_, ()>(()))
                } else {
                    std::task::Poll::Pending
                }
            })
        });
        tokio::pin!(run);
        assert!(
            std::future::poll_fn(|cx| std::task::Poll::Ready(run.as_mut().poll(cx)))
                .await
                .is_pending()
        );
        ready.set(true);
        tokio::time::advance(Duration::from_secs(1)).await;
        if cancel {
            owner.cancel();
        }
        let expected = if cancel {
            Interruption::Cancelled
        } else {
            Interruption::DeadlineExceeded
        };
        assert!(
            matches!(run.await, Err(TrackedOperationError::Interrupted(reason)) if reason == expected)
        );
        assert_eq!(polls.get(), 1, "expired work must not be polled again");
    }
}

#[tokio::test(start_paused = true)]
async fn borrowed_non_send_results_and_errors_remain_concrete() {
    struct DomainError(Rc<u8>);
    let owner = OperationOwner::new(Duration::from_secs(1)).unwrap();
    let value = Rc::new(42);
    let output = owner
        .context()
        .run_tracked("value", |_| async { Ok::<_, DomainError>(&value) })
        .await
        .unwrap();
    assert!(Rc::ptr_eq(output, &value));
    let error = owner
        .context()
        .run_tracked("error", |_| async {
            Err::<(), _>(DomainError(value.clone()))
        })
        .await
        .unwrap_err();
    assert_eq!(format!("{error:?}"), "Failed([REDACTED])");
    assert_eq!(error.to_string(), "operation failed");
    match error {
        TrackedOperationError::Failed(DomainError(retained)) => {
            assert!(Rc::ptr_eq(&retained, &value))
        }
        other => panic!("wrong outcome: {other:?}"),
    }
}

#[tokio::test(start_paused = true)]
async fn borrowed_wait_can_resume_but_dropping_the_run_cancels_its_scope() {
    let owner = OperationOwner::new(Duration::from_secs(10)).unwrap();
    let child = std::cell::RefCell::new(None);
    let mut run = Box::pin(owner.context().run_tracked("resume", |scope| {
        *child.borrow_mut() = Some(scope);
        pending::<Result<(), ()>>()
    }));
    assert!(
        tokio::time::timeout(Duration::from_secs(1), &mut run)
            .await
            .is_err()
    );
    assert!(child.borrow().as_ref().unwrap().check().is_ok());
    assert!(
        std::future::poll_fn(|cx| std::task::Poll::Ready(run.as_mut().poll(cx)))
            .await
            .is_pending()
    );
    drop(run);
    assert_eq!(
        child.borrow().as_ref().unwrap().check(),
        Err(Interruption::Cancelled)
    );
    assert!(owner.context().check().is_ok());
}

#[tokio::test]
async fn unpolled_run_is_inert_and_completed_scope_is_cancelled() {
    let owner = OperationOwner::new(Duration::from_secs(10)).unwrap();
    let unpolled = owner.context().run_tracked("unpolled", |_| {
        panic!("unpolled factory invoked");
        #[expect(unreachable_code, reason = "specifies the future type")]
        pending::<Result<(), ()>>()
    });
    drop(unpolled);
    let child = owner
        .context()
        .run_tracked("complete", |scope| async { Ok::<_, ()>(scope) })
        .await
        .unwrap();
    assert_eq!(child.check(), Err(Interruption::Cancelled));
    assert!(owner.context().check().is_ok());
}

#[tokio::test(start_paused = true)]
async fn absolute_children_clamp_and_accept_elapsed_instants() {
    let owner = OperationOwner::new(Duration::from_secs(10)).unwrap();
    let now = Instant::now();
    for requested in [
        now - Duration::from_secs(1),
        now,
        now + Duration::from_secs(3),
        now + Duration::from_secs(30),
    ] {
        let child = owner.context().child_until(requested);
        assert_eq!(
            child.context().deadline(),
            requested.min(owner.context().deadline())
        );
        if requested <= now {
            assert_eq!(child.context().check(), Err(Interruption::DeadlineExceeded));
        }
        child.cancel();
        assert!(owner.context().check().is_ok());
    }
    let child = owner.context().child_until(now + Duration::from_secs(3));
    let grandchild = child.context().child_until(now + Duration::from_secs(5));
    assert_eq!(grandchild.context().deadline(), child.context().deadline());
    tokio::time::advance(Duration::from_secs(11)).await;
    let late_child = owner
        .context()
        .child_until(Instant::now() + Duration::from_secs(30));
    assert_eq!(late_child.context().deadline(), owner.context().deadline());
    assert_eq!(
        late_child.context().check(),
        Err(Interruption::DeadlineExceeded)
    );
    owner.cancel();
    assert_eq!(grandchild.context().check(), Err(Interruption::Cancelled));
    assert_eq!(
        owner.context().child_until(now).context().check(),
        Err(Interruption::Cancelled)
    );
}

#[tokio::test]
async fn factory_and_future_panics_propagate_and_cancel_the_child_scope() {
    for panic_in_factory in [true, false] {
        let owner = OperationOwner::new(Duration::from_secs(10)).unwrap();
        let context = owner.context().clone();
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            context
                .run_tracked("panic", |scope| {
                    sender.send(scope).unwrap();
                    assert!(!panic_in_factory, "factory panic");
                    async {
                        panic!("future panic");
                        #[expect(unreachable_code, reason = "specifies the result type")]
                        Ok::<(), ()>(())
                    }
                })
                .await
        });
        assert!(task.await.unwrap_err().is_panic());
        assert_eq!(
            receiver.await.unwrap().check(),
            Err(Interruption::Cancelled)
        );
        assert!(owner.context().check().is_ok());
    }
}
