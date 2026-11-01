//! Child-side scenarios. Each runs in a fresh process, so SIGTERM and SIGINT
//! start with their default dispositions and only the scenario can replace them.

use batter_core::{
    BoxError, RegistrationError,
    cleanup::{CleanupBudget, CleanupOutcome, CleanupReport},
    command::{
        Command, CommandCause, CommandFailure, CommandOwner, CommandReport, CommandScope,
        SharedCommandReport,
    },
    operation::{Interruption, OperationContext, OperationOwner},
};
use std::{
    error::Error,
    fmt,
    future::pending,
    io::Write,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};
use tokio::{
    runtime::{Builder, Runtime},
    signal::unix::{SignalKind, signal},
};

/// Bounds a stalled child only; the launch emergency exit fires first.
const WORK_BUDGET: Duration = Duration::from_secs(60);
/// Failure guard for in-child waits on the parent's signal.
const GUARD: Duration = Duration::from_secs(5);
const AUDIT_TEXT: &str = "audit record rejected by the archive";

pub(crate) fn run(scenario: &str) {
    match scenario {
        "before-work" => before_work(),
        "during-work" => during_work(current_thread()),
        "during-work-multi-thread" => during_work(multi_thread()),
        "during-cleanup" => current_thread().block_on(during_cleanup()),
        "repeated" => current_thread().block_on(repeated()),
        "never-started" => current_thread().block_on(never_started()),
        "without-signals" => current_thread().block_on(without_signals()),
        _ => panic!("unknown command signal scenario"),
    }
}

/// One parseable record per line. The leading newline separates it from
/// libtest's unterminated `test child_fixture ...` prefix.
fn mark(event: &str) {
    let mut stdout = std::io::stdout().lock();
    writeln!(stdout, "\nbatter-fixture:{event}").unwrap();
    stdout.flush().unwrap();
}

fn current_thread() -> Runtime {
    Builder::new_current_thread().enable_all().build().unwrap()
}

fn multi_thread() -> Runtime {
    Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap()
}

fn cleanup_budget() -> CleanupBudget {
    CleanupBudget::new(GUARD, GUARD, Duration::from_secs(1)).unwrap()
}

fn root() -> OperationOwner {
    OperationOwner::new(WORK_BUDGET).unwrap()
}

/// The parent context is supplied to the command and must never be cancelled by it.
fn assert_live(parent: &OperationContext) {
    assert_eq!(
        parent.check(),
        Ok(()),
        "a signal reached the parent context"
    );
}

async fn failed<T, E>(owner: CommandOwner<T, E>) -> SharedCommandReport<T, E> {
    match owner.finish().await {
        Err(CommandFailure::Report(report)) => report,
        Err(CommandFailure::Coordinator(_)) => panic!("command coordinator ended without a report"),
        Ok(_) => panic!("a signalled command reported success"),
    }
}

fn assert_signal_cancelled<T, E>(report: &CommandReport<T, E>) -> &CleanupReport {
    assert!(matches!(
        report.work,
        Err(CommandCause::Interrupted(Interruption::Cancelled))
    ));
    assert!(report.interruption_after_work.is_none());
    assert!(report.destruction_panic.is_none());
    report
        .cleanup
        .as_ref()
        .expect("cleanup coordinator completed")
}

fn assert_only_probe_succeeded(cleanup: &CleanupReport) {
    let records: Vec<_> = cleanup
        .records
        .iter()
        .map(|record| (record.name, record.outcome))
        .collect();
    // Exactly one run: a later signal neither restarts nor times out cleanup.
    assert_eq!(records, [("probe", CleanupOutcome::Succeeded)]);
    assert!(cleanup.skipped.is_empty() && cleanup.is_success());
}

/// C1: the oracle's runtime performs Tokio's process-wide broadcast while the
/// command's runtime is blocked, so the signal is visible to the command's
/// listener before its coordinator is ever polled.
fn before_work() {
    let (installed, oracle_installed) = mpsc::channel();
    let (observed, oracle_observed) = mpsc::channel();
    let oracle = thread::spawn(move || {
        current_thread().block_on(async move {
            let mut terminate = signal(SignalKind::terminate()).unwrap();
            let mut interrupt = signal(SignalKind::interrupt()).unwrap();
            installed.send(()).unwrap();
            tokio::select! {
                _ = terminate.recv() => {}
                _ = interrupt.recv() => {}
            }
            observed.send(()).unwrap();
        });
    });
    oracle_installed
        .recv_timeout(GUARD)
        .expect("oracle listeners installed");
    let parent = root();
    let invoked = Arc::new(AtomicBool::new(false));
    let invoking = invoked.clone();
    current_thread().block_on(async {
        let owner = Command::new(parent.context().clone(), cleanup_budget(), move |_scope| {
            invoking.store(true, Ordering::SeqCst);
            mark("factory-invoked");
            Box::pin(async { Ok::<_, BoxError>(()) })
        })
        .start_with_unix_signals()
        .expect("command listeners install");
        mark("before-work-ready");
        // Block this runtime's only thread without yielding: the coordinator
        // cannot run until the oracle has observed the parent's signal.
        oracle_observed
            .recv_timeout(GUARD)
            .expect("oracle observed the signal");
        let report = failed(owner).await;
        let cleanup = assert_signal_cancelled(&report);
        // Proves both synchronous installation and the pre-factory check.
        assert!(!invoked.load(Ordering::SeqCst), "the factory ran");
        assert!(cleanup.records.is_empty() && cleanup.is_success());
        assert_live(parent.context());
    });
    oracle.join().unwrap();
    mark("before-work-checked");
}

#[derive(Debug)]
struct AuditFailure;

impl fmt::Display for AuditFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(AUDIT_TEXT)
    }
}

impl Error for AuditFailure {}

/// C2: the signal cancels pending work; LIFO cleanup still runs completely.
fn during_work(runtime: Runtime) {
    let parent = root();
    runtime.block_on(async {
        let owner = Command::new(parent.context().clone(), cleanup_budget(), |scope| {
            Box::pin(async move {
                let descendant = scope.context().child(WORK_BUDGET)?.into_context();
                scope
                    .reserve_cleanup("resource")?
                    .register(move || async move {
                        // Captured only to observe the cancellation's downward reach.
                        assert_eq!(descendant.check(), Err(Interruption::Cancelled));
                        mark("resource-released");
                        Ok(())
                    });
                scope
                    .reserve_cleanup("audit")?
                    .register(|| async { Err(Box::new(AuditFailure) as BoxError) });
                mark("work-started");
                pending::<Result<(), BoxError>>().await
            })
        })
        .start_with_unix_signals()
        .expect("command listeners install");
        let report = failed(owner).await;
        assert_audited_cleanup(assert_signal_cancelled(&report));
        assert_live(parent.context());
        assert!(!format!("{report:?}").contains(AUDIT_TEXT));
    });
    mark("during-work-checked");
}

fn assert_audited_cleanup(cleanup: &CleanupReport) {
    let names: Vec<_> = cleanup.records.iter().map(|record| record.name).collect();
    assert_eq!(names, ["audit", "resource"]);
    let audit = &cleanup.records[0];
    assert_eq!(audit.outcome, CleanupOutcome::Failed);
    let error = audit.error.as_ref().expect("audit error retained");
    assert!(error.downcast_ref::<AuditFailure>().is_some());
    assert_eq!(cleanup.records[1].outcome, CleanupOutcome::Succeeded);
    assert!(cleanup.skipped.is_empty());
    assert!(!format!("{cleanup:?}").contains(AUDIT_TEXT));
}

/// A finalizer that owns its own listeners, so it can report the parent's
/// signal; the command's listeners were dropped before cleanup began.
fn register_probe(scope: &mut CommandScope) -> Result<(), RegistrationError> {
    scope.reserve_cleanup("probe")?.register(|| async {
        let mut terminate = signal(SignalKind::terminate())?;
        let mut interrupt = signal(SignalKind::interrupt())?;
        mark("cleanup-started");
        tokio::select! {
            _ = terminate.recv() => {}
            _ = interrupt.recv() => {}
        }
        mark("cleanup-saw-signal");
        Ok::<(), BoxError>(())
    });
    Ok(())
}

/// Deliberately without derives: the checked value is moved out, never copied.
struct Exported {
    rows: u32,
}

/// C3: a signal during cleanup neither interrupts nor fails a successful command.
async fn during_cleanup() {
    let parent = root();
    let owner = Command::new(parent.context().clone(), cleanup_budget(), |scope| {
        Box::pin(async move {
            register_probe(scope)?;
            Ok::<_, BoxError>(Exported { rows: 7 })
        })
    })
    .start_with_unix_signals()
    .expect("command listeners install");
    let success = owner
        .finish()
        .await
        .expect("a cleanup-phase signal left the command successful");
    assert_only_probe_succeeded(success.cleanup());
    assert_eq!(success.stage(), "command");
    let exported: Exported = success.into_value();
    assert_eq!(exported.rows, 7);
    assert_live(parent.context());
    mark("during-cleanup-checked");
}

/// C4: the first signal cancels work; the second reaches only running cleanup.
async fn repeated() {
    let parent = root();
    let owner = Command::new(parent.context().clone(), cleanup_budget(), |scope| {
        Box::pin(async move {
            register_probe(scope)?;
            mark("work-started");
            pending::<Result<(), BoxError>>().await
        })
    })
    .start_with_unix_signals()
    .expect("command listeners install");
    let report = failed(owner).await;
    assert_only_probe_succeeded(assert_signal_cancelled(&report));
    assert_live(parent.context());
    mark("repeated-checked");
}

/// C5a: an inert specification installs nothing.
async fn never_started() {
    let _unstarted = Command::new(root().into_context(), cleanup_budget(), |_scope| {
        Box::pin(async { Ok::<_, BoxError>(()) })
    });
    mark("never-started-ready");
    pending::<()>().await;
}

/// C5b: the unsignalled owner path installs nothing while work runs.
async fn without_signals() {
    let _owner = Command::new(root().into_context(), cleanup_budget(), |_scope| {
        Box::pin(async {
            mark("work-started");
            pending::<Result<(), BoxError>>().await
        })
    })
    .start_without_signals();
    pending::<()>().await;
}
