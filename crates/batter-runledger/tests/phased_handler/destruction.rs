use super::*;
use std::{pin::Pin, task::Poll};

struct Returned<T> {
    result: Option<Result<T, &'static str>>,
    destroyed: Arc<AtomicBool>,
}
impl<T: Unpin> Future for Returned<T> {
    type Output = Result<T, &'static str>;
    fn poll(mut self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Self::Output> {
        Poll::Ready(self.result.take().unwrap())
    }
}
impl<T> Drop for Returned<T> {
    fn drop(&mut self) {
        self.destroyed.store(true, Ordering::SeqCst);
        panic!("secret callback destruction");
    }
}
struct DestructionProbe {
    fails: bool,
    work_destroyed: Arc<AtomicBool>,
    final_destroyed: Arc<AtomicBool>,
    failure: Arc<Mutex<Option<Failure>>>,
}
// Spell out async_trait's future ABI so the callback itself can panic on Drop
// after returning. An async-body guard would panic before returning its value.
impl PhasedJobHandler for DestructionProbe {
    type Output = usize;
    type Error = &'static str;
    fn job_type(&self) -> JobType<'static> {
        JobType::new("test.phase_destruction")
    }
    fn work<'a, 's, 'f>(
        &'a self,
        _: JobWork<'s>,
        _: Value,
    ) -> Pin<Box<dyn Future<Output = Result<usize, &'static str>> + Send + 'f>>
    where
        'a: 'f,
        's: 'f,
        Self: 'f,
    {
        Box::pin(Returned {
            result: Some(if self.fails { Err("work error") } else { Ok(7) }),
            destroyed: self.work_destroyed.clone(),
        })
    }
    fn finalize<'a, 's, 'f>(
        &'a self,
        scope: JobFinalization<'s, usize, &'static str>,
    ) -> Pin<Box<dyn Future<Output = Result<JobCompletion, &'static str>> + Send + 'f>>
    where
        'a: 'f,
        's: 'f,
        Self: 'f,
    {
        assert!(self.work_destroyed.load(Ordering::SeqCst));
        assert!(
            scope
                .work()
                .classify()
                .unwrap_err()
                .destruction_panic()
                .is_some()
        );
        Box::pin(Returned {
            result: Some(if self.fails {
                Err("final error")
            } else {
                Ok(JobCompletion::success())
            }),
            destroyed: self.final_destroyed.clone(),
        })
    }
    fn failed(&self, failure: Failure) -> JobFailure {
        *self.failure.lock().unwrap() = Some(failure);
        JobFailure::terminal("test.phase_destruction", "Both outcomes retained.")
    }
}

#[tokio::test]
async fn native_adapter_retains_callback_returns_and_destruction_panics() {
    for fails in [false, true] {
        let failure = Arc::new(Mutex::new(None));
        let final_destroyed = Arc::new(AtomicBool::new(false));
        let handler = PhasedJob::new(
            DestructionProbe {
                fails,
                failure: failure.clone(),
                work_destroyed: Arc::default(),
                final_destroyed: final_destroyed.clone(),
            },
            Duration::from_secs(1),
        )
        .unwrap();
        let run = HandlerInvocation::new(Duration::from_secs(3))
            .unwrap()
            .execute(&handler, Value::Null)
            .await;
        assert!(
            matches!(run.exit, HandlerExit::Returned(Err(ref e)) if e.code == "test.phase_destruction")
        );
        assert!(final_destroyed.load(Ordering::SeqCst));
        let Some(PhasedJobFailure::Phases(PhasedFailure::Report(report))) =
            failure.lock().unwrap().take()
        else {
            panic!("both outcomes retained");
        };
        let work = report.work().classify().unwrap_err();
        let finalization = report.finalization().classify().unwrap_err();
        assert!(work.destruction_panic().is_some());
        assert!(finalization.destruction_panic().is_some());
        if fails {
            assert!(matches!(
                work.returned_result(),
                Err(CommandCause::Failed("work error"))
            ));
            assert!(matches!(
                finalization.returned_result(),
                Err(CommandCause::Failed("final error"))
            ));
        } else {
            assert_eq!(work.returned_result().unwrap(), &7);
            assert_eq!(
                finalization.returned_result().unwrap().disposition(),
                runledger_core::jobs::JobCompletionDisposition::Succeed
            );
        }
        assert!(!format!("{report:?}").contains("secret"));
    }
}
