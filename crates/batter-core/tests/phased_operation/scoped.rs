use batter_core::{
    command::CommandCause,
    operation::{
        Interruption, OperationContext, OperationOwner, PhaseOutcome, PhasedFailure,
        low_level::{ScopedPhases, drive_phases},
    },
};
use std::{
    future::Future,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll},
    time::Duration,
};

struct Returned {
    result: Option<Result<usize, &'static str>>,
    destroyed: Arc<AtomicBool>,
}
impl Future for Returned {
    type Output = Result<usize, &'static str>;
    fn poll(mut self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Self::Output> {
        Poll::Ready(self.result.take().expect("polled once"))
    }
}
impl Drop for Returned {
    fn drop(&mut self) {
        self.destroyed.store(true, Ordering::SeqCst);
        panic!("secret destructor payload");
    }
}
struct Driver {
    fails: bool,
    destroyed: Arc<AtomicBool>,
    work_context: Option<OperationContext>,
}
impl ScopedPhases for Driver {
    type Work = usize;
    type WorkError = &'static str;
    type Final = usize;
    type FinalError = &'static str;

    fn work(
        &mut self,
        context: &OperationContext,
    ) -> impl Future<Output = Result<usize, &'static str>> + Send {
        self.work_context = Some(context.clone());
        Returned {
            result: Some(if self.fails { Err("work error") } else { Ok(7) }),
            destroyed: self.destroyed.clone(),
        }
    }
    fn finalize(
        &mut self,
        context: &OperationContext,
        work: &PhaseOutcome<usize, &'static str>,
    ) -> impl Future<Output = Result<usize, &'static str>> + Send {
        assert!(
            self.destroyed.load(Ordering::SeqCst),
            "work destroyed before finalization"
        );
        assert_eq!(
            self.work_context.as_ref().expect("work ran").check(),
            Err(Interruption::Cancelled),
            "work context cancelled before finalization"
        );
        assert!(context.check().is_ok(), "finalization remains live");
        assert!(
            work.classify()
                .expect_err("destruction failed")
                .destruction_panic()
                .is_some(),
            "work destruction evidence retained"
        );
        Returned {
            result: Some(if self.fails {
                Err("final error")
            } else {
                Ok(9)
            }),
            destroyed: Arc::default(),
        }
    }
}

#[tokio::test]
async fn scoped_callbacks_retain_returned_values_and_errors_across_destruction_panics() {
    for fails in [false, true] {
        let owner = OperationOwner::new(Duration::from_secs(10)).unwrap();
        let phases = owner
            .context()
            .reserve_finalization(Duration::from_secs(2))
            .unwrap();
        let mut driver = Driver {
            fails,
            destroyed: Arc::default(),
            work_context: None,
        };
        let Err(PhasedFailure::Report(report)) = drive_phases(phases, &mut driver).await else {
            panic!("destruction failure must prevent success");
        };
        for (phase, value, error) in [
            (report.work(), 7, "work error"),
            (report.finalization(), 9, "final error"),
        ] {
            let failure = phase.classify().unwrap_err();
            assert!(failure.destruction_panic().is_some());
            assert_eq!(failure.interruption_after_return(), None);
            if fails {
                assert!(
                    matches!(failure.returned_result(), Err(CommandCause::Failed(e)) if *e == error)
                );
            } else {
                assert_eq!(failure.returned_result().unwrap(), &value);
            }
        }
        assert!(!format!("{report:?}").contains("secret"));
    }
}
