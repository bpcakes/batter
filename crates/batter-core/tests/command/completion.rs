use super::*;
use std::{pin::Pin, sync::Mutex, task::Context};

type CommandOwner = Arc<Mutex<Option<batter_core::command::RunningCommand<u32, &'static str>>>>;

#[derive(Clone, Copy)]
enum Interrupt {
    Cancel,
    Expire,
}

impl Interrupt {
    fn apply(self, context: &OperationContext, owner: &CommandOwner, cx: &mut Context<'_>) {
        match self {
            Self::Cancel => owner.lock().unwrap().as_ref().unwrap().cancel(),
            Self::Expire => {
                let mut advancing = Box::pin(tokio::time::advance(Duration::from_secs(11)));
                let _ = advancing.as_mut().poll(cx);
                assert_eq!(context.check(), Err(Interruption::DeadlineExceeded));
            }
        }
    }

    fn reason(self) -> Interruption {
        match self {
            Self::Cancel => Interruption::Cancelled,
            Self::Expire => Interruption::DeadlineExceeded,
        }
    }
}

struct Completing {
    context: OperationContext,
    owner: CommandOwner,
    interrupt: Interrupt,
    during_poll: bool,
    fail: bool,
}

impl Future for Completing {
    type Output = Result<u32, &'static str>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if self.during_poll {
            self.interrupt.apply(&self.context, &self.owner, cx);
        }
        Poll::Ready(if self.fail {
            Err("original failure")
        } else {
            Ok(42)
        })
    }
}

impl Drop for Completing {
    fn drop(&mut self) {
        if !self.during_poll {
            self.interrupt.apply(
                &self.context,
                &self.owner,
                &mut Context::from_waker(std::task::Waker::noop()),
            );
        }
    }
}

async fn completion_boundary(interrupt: Interrupt, during_poll: bool) {
    for fail in [false, true] {
        let closed = Arc::new(AtomicBool::new(false));
        let closing = closed.clone();
        let parent = context();
        let owner: CommandOwner = Arc::new(Mutex::new(None));
        let work_owner = Arc::clone(&owner);
        let command = Command::new(parent.clone(), budget(), move |scope| {
            scope
                .reserve_cleanup("resource")
                .unwrap()
                .register(move || async move {
                    closing.store(true, Ordering::SeqCst);
                    Ok(())
                });
            Box::pin(Completing {
                context: scope.context().clone(),
                owner: work_owner,
                interrupt,
                during_poll,
                fail,
            })
        })
        .start();
        let observer = command.observer();
        *owner.lock().unwrap() = Some(command);
        let report = observer.wait().await.unwrap();
        assert_eq!(
            report.interruption_after_work,
            during_poll.then(|| interrupt.reason())
        );
        if fail {
            assert!(matches!(
                report.work,
                Err(CommandCause::Failed("original failure"))
            ));
        } else {
            assert_eq!(*report.work.as_ref().unwrap(), 42);
        }
        assert_eq!(report.is_success(), !during_poll && !fail);
        assert!(report.destruction_panic.is_none());
        assert!(report.cleanup.as_ref().unwrap().is_success());
        assert!(closed.load(Ordering::SeqCst));
        assert_ne!(parent.check(), Err(Interruption::Cancelled));
    }
}

#[tokio::test(start_paused = true)]
async fn cancellation_in_final_poll_is_retained() {
    completion_boundary(Interrupt::Cancel, true).await;
}

#[tokio::test(start_paused = true)]
async fn deadline_in_final_poll_is_retained() {
    completion_boundary(Interrupt::Expire, true).await;
}

#[tokio::test(start_paused = true)]
async fn cancellation_in_destruction_does_not_reclassify_completed_work() {
    completion_boundary(Interrupt::Cancel, false).await;
}

#[tokio::test(start_paused = true)]
async fn deadline_in_destruction_does_not_reclassify_completed_work() {
    completion_boundary(Interrupt::Expire, false).await;
}
