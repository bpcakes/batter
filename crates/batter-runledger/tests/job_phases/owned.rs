use super::*;
use batter_core::{command::CommandCause, operation::PhasedFailure};
use std::{convert::Infallible, future::pending};

#[tokio::test(start_paused = true)]
async fn owned_conversion_keeps_native_deadline_and_exit_cancellation() {
    for reserve in [Duration::ZERO, RESERVE] {
        let services = Services::new();
        let context = context();
        let phases = job_phases(JobExecution::new(&context, &services), reserve).unwrap();
        let deadline = services.deadline();
        let (started, ready) = tokio::sync::oneshot::channel();
        let finalized = Arc::new(AtomicUsize::new(0));
        let count = finalized.clone();
        advance(Duration::from_secs(1)).await;
        let running = phases
            .into_operation(
                async move |work| {
                    assert_eq!(work.deadline(), deadline - reserve);
                    started.send(()).unwrap();
                    pending::<Result<(), Infallible>>().await
                },
                async move |_scope| {
                    count.fetch_add(1, Ordering::SeqCst);
                    Ok::<_, Infallible>(())
                },
            )
            .start();
        ready.await.unwrap();
        services.end();
        let PhasedFailure::Report(report) = running.wait().await.unwrap_err() else {
            panic!("invocation end must retain both phase outcomes");
        };
        for phase in [report.work(), report.finalization()] {
            assert!(matches!(
                phase.classify().unwrap_err().returned_result(),
                Err(CommandCause::Interrupted(Interruption::Cancelled))
            ));
        }
        assert_eq!(finalized.load(Ordering::SeqCst), 0);
    }
}
