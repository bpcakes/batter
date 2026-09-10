use super::support::{Case, supervisor, yields};
use batter::{
    BoxError,
    cleanup::CleanupOutcome,
    lifecycle::{ProcessAdmissionError, ProcessTaskError, ShutdownReport, TaskOutcome},
};
use std::{
    error::Error,
    fmt,
    io::{self, Write},
    sync::{Arc, Mutex},
};
use tokio::sync::{Semaphore, mpsc, oneshot};
use tracing::instrument::WithSubscriber;

struct Cause(u64);
impl fmt::Display for Cause {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("private-scheduling-cause")
    }
}
impl fmt::Debug for Cause {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
impl Error for Cause {}

#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<u8>>>);
impl Write for Capture {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let mut output = self.0.lock().unwrap();
        assert!(
            output.len() + bytes.len() <= 131_072,
            "scoped diagnostic capture overflow"
        );
        output.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn cause<'a>(error: &'a (dyn Error + 'static)) -> &'a Cause {
    if let Some(cause) = error.downcast_ref::<Cause>() {
        return cause;
    }
    cause(error.source().expect("missing retained cause"))
}

pub async fn retained_errors(case: Case, delay: u64) {
    let output = Capture::default();
    let writer = output.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_ansi(false)
        .without_time()
        .with_max_level(tracing::Level::INFO)
        .finish();
    errors_inner(case, delay)
        .with_subscriber(crate::test_dispatch::new(subscriber))
        .await;
    let bytes = output.0.lock().unwrap();
    let text = std::str::from_utf8(&bytes).unwrap();
    assert!(text.contains("task"));
    assert!(text.contains("cleanup"));
    assert!(
        !text.contains("private-scheduling-cause"),
        "returned error contents leaked"
    );
    case.event("all-causes-retained-diagnostics-redacted");
}

async fn errors_inner(case: Case, delay: u64) {
    let mut supervisor = supervisor(3);
    let gate = Arc::new(Semaphore::new(0));
    let (started, mut ready) = mpsc::channel(4);
    for (id, name) in [(0, "critical-first"), (1, "critical-second")] {
        let gate = gate.clone();
        let started = started.clone();
        supervisor
            .register(name, move |signal| async move {
                signal.mark_started();
                started.send(()).await.unwrap();
                let _permit = gate.acquire().await.unwrap();
                yields(delay.wrapping_add(id)).await;
                Err(Box::new(Cause(id)) as BoxError)
            })
            .unwrap();
    }
    for (id, name) in [(4, "cleanup-first"), (5, "cleanup-second")] {
        supervisor
            .on_cleanup(
                name,
                move || async move { Err(Box::new(Cause(id)) as BoxError) },
            )
            .unwrap();
    }
    let process = supervisor.process_handle().unwrap();
    let running = supervisor.start();
    running.handle().wait_ready().await.unwrap();
    let (scope_tx, scope_rx) = oneshot::channel();
    let (release, released) = oneshot::channel();
    let parent = process
        .try_spawn("active-ancestor", |scope| async move {
            scope_tx.send(scope).ok().unwrap();
            released.await.unwrap();
            Ok::<_, std::convert::Infallible>(())
        })
        .unwrap();
    let scope = scope_rx.await.unwrap();
    let mut receipts = Vec::new();
    for (id, name) in [(2, "finite-first"), (3, "finite-second")] {
        let gate = gate.clone();
        let started = started.clone();
        receipts.push(
            process
                .try_spawn(name, move |_| async move {
                    started.send(()).await.unwrap();
                    let _permit = gate.acquire().await.unwrap();
                    yields(delay.wrapping_add(id)).await;
                    Err::<(), _>(Cause(id))
                })
                .unwrap(),
        );
    }
    for _ in 0..4 {
        ready.recv().await.unwrap();
    }
    // All errors were admitted before any failure can close admission.
    case.event("four-failures-parked");
    gate.add_permits(4);
    drop(receipts.pop().unwrap());
    let original = match receipts.pop().unwrap().wait().await {
        Err(ProcessTaskError::Failed(error)) => error,
        _ => panic!("typed task failure lost"),
    };
    assert!(matches!(
        scope.try_spawn("after-failure", |_| async {
            Ok::<_, std::convert::Infallible>(())
        }),
        Err(ProcessAdmissionError::Closed)
    ));
    release.send(()).unwrap();
    parent.wait().await.unwrap();
    let report = running.wait().await.unwrap();
    verify_retained(&report, &original);
}

fn verify_retained(report: &ShutdownReport, original: &Arc<Cause>) {
    assert!(!report.is_success());
    assert_eq!(report.completed_process_tasks, 1);
    assert_eq!(report.tasks.len(), 4);
    let mut ids = Vec::new();
    for task in &report.tasks {
        assert_eq!(task.outcome, TaskOutcome::Failed);
        let retained = cause(task.error.as_deref().unwrap());
        assert_eq!(
            task.name,
            [
                "critical-first",
                "critical-second",
                "finite-first",
                "finite-second"
            ][retained.0 as usize]
        );
        if retained.0 == 2 {
            assert!(std::ptr::eq(original.as_ref(), retained));
        }
        ids.push(retained.0);
    }
    ids.sort_unstable();
    assert_eq!(ids, [0, 1, 2, 3]);
    assert_eq!(report.cleanup.records.len(), 2);
    for (record, (id, name)) in report
        .cleanup
        .records
        .iter()
        .zip([(5, "cleanup-second"), (4, "cleanup-first")])
    {
        assert_eq!(record.name, name);
        assert_eq!(record.outcome, CleanupOutcome::Failed);
        assert_eq!(cause(record.error.as_deref().unwrap()).0, id);
    }
    assert!(report.cleanup.skipped.is_empty());
    assert!(report.abort_requested.is_empty() && report.unjoined.is_empty());
}
