//! Child-side scenarios. Each runs in a fresh process that starts with Rust's
//! default panic hook; only the scenario replaces it.

use batter_core::{
    BoxError,
    admission::{Admission, BlockingError, Bulkhead, BulkheadCapacity},
    cleanup::CleanupBudget,
    command::{Command, CommandCause, CommandFailure},
    lifecycle::{ShutdownBudget, Supervisor},
    operation::{OperationContext, OperationOwner},
    startup::{ApplicationError, Startup, StartupCause, StartupError},
};
use std::{
    any::Any,
    fs::File,
    io::Write,
    os::fd::AsFd,
    panic::{self, Location, PanicHookInfo, catch_unwind},
    ptr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    thread::{self, ThreadId},
    time::Duration,
};
use tokio::runtime::Builder;

/// A secret-shaped value carried by every payload passed to `leak`.
pub(crate) const SECRET: &str = "postgres://fixture:panic-secret-6f1c@localhost/database";
/// The secret's distinctive core, which `Debug` string escaping leaves intact.
pub(crate) const SECRET_CORE: &str = "panic-secret-6f1c";
/// Bounds a stalled scenario only. Each context's budget starts after launch,
/// so the parent's equal guard normally ends a stalled child first, and the
/// 10 s launch emergency exit is only a backstop.
const BUDGET: Duration = Duration::from_secs(5);
/// `EMFILE` on Linux and macOS.
const TOO_MANY_OPEN_FILES: i32 = 24;

pub(crate) fn run(scenario: &str) {
    match scenario {
        "default-hook" => caught(),
        "caught" => {
            install();
            caught();
        }
        "uncaught-default" => uncaught(),
        "uncaught" => {
            install();
            uncaught();
        }
        "tokio-current-thread" => {
            install();
            tokio_task(Flavor::CurrentThread);
        }
        "tokio-multi-thread" => {
            install();
            tokio_task(Flavor::MultiThread);
        }
        "std-thread" => {
            install();
            std_thread();
        }
        "stderr-lock-held" => {
            install();
            stderr_lock_held();
        }
        "descriptor-limit" => {
            install();
            descriptor_limit();
        }
        "entrypoints" => entrypoints(),
        _ => panic!("unknown panic hook scenario"),
    }
}

/// One parseable record per line. The leading newline separates it from
/// libtest's unterminated `test child_fixture ...` prefix.
pub(crate) fn mark(event: &str) {
    let mut stdout = std::io::stdout().lock();
    writeln!(stdout, "\nbatter-fixture:{event}").unwrap();
    stdout.flush().unwrap();
}

/// The application root's first statement.
fn install() {
    panic::set_hook(batter_core::redacting_panic_hook());
    mark("hook-installed");
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Payload {
    /// A constant `panic!` message, which std keeps as a `&'static str`.
    Literal,
    /// A formatted `panic!` message.
    Message,
    /// `unwrap` on an error whose `Debug` holds the secret.
    UnwrapDebug,
    /// A non-string payload from `panic_any`. The default hook prints no
    /// non-string payload, so this kind is location and integrity evidence only.
    Opaque,
}

#[derive(Debug, PartialEq, Eq)]
struct Credential(&'static str);

/// A native call whose error carries the credential in its `Debug`.
fn connect() -> Result<(), Credential> {
    Err(Credential(SECRET))
}

/// Panic with the secret at the caller's location, after recording that
/// location so the parent can require it on standard error.
#[track_caller]
fn leak(payload: Payload) -> ! {
    let site = Location::caller();
    mark(&format!(
        "site:{}:{}:{}",
        site.file(),
        site.line(),
        site.column()
    ));
    match payload {
        // The literal must equal `SECRET`; `assert_payload` checks that.
        Payload::Literal => panic!("postgres://fixture:panic-secret-6f1c@localhost/database"),
        Payload::Message => panic!("connection failed: {SECRET}"),
        Payload::UnwrapDebug => {
            connect().unwrap();
            unreachable!("unwrap of an error returned")
        }
        Payload::Opaque => panic::panic_any(Credential(SECRET)),
    }
}

/// Proves that a frame was unwound: it marks only from its destructor.
struct Unwound(&'static str);

impl Drop for Unwound {
    fn drop(&mut self) {
        if thread::panicking() {
            mark(&format!("unwound:{}", self.0));
        }
    }
}

/// The payload survived the hook unchanged, so the hook neither consumed
/// nor replaced it, and it carried the secret the parent must not see.
fn assert_payload(payload: &(dyn Any + Send), expected: Payload) {
    let intact = match expected {
        Payload::Literal => payload.downcast_ref::<&'static str>() == Some(&SECRET),
        Payload::Message => payload
            .downcast_ref::<String>()
            .is_some_and(|message| *message == format!("connection failed: {SECRET}")),
        Payload::UnwrapDebug => payload
            .downcast_ref::<String>()
            .is_some_and(|message| message.contains(SECRET_CORE)),
        Payload::Opaque => payload.downcast_ref::<Credential>() == Some(&Credential(SECRET)),
    };
    assert!(intact, "{expected:?} payload changed");
}

fn caught() {
    let literal = catch_unwind(|| {
        let _frame = Unwound("literal");
        leak(Payload::Literal)
    });
    let message = catch_unwind(|| {
        let _frame = Unwound("message");
        leak(Payload::Message)
    });
    let unwrap_debug = catch_unwind(|| {
        let _frame = Unwound("unwrap-debug");
        leak(Payload::UnwrapDebug)
    });
    let opaque = catch_unwind(|| {
        let _frame = Unwound("opaque");
        leak(Payload::Opaque)
    });
    assert_payload(&*literal.expect_err("panicked"), Payload::Literal);
    assert_payload(&*message.expect_err("panicked"), Payload::Message);
    assert_payload(&*unwrap_debug.expect_err("panicked"), Payload::UnwrapDebug);
    assert_payload(&*opaque.expect_err("panicked"), Payload::Opaque);
    mark("payloads-intact");
}

/// Unwinds out of the libtest test function, which exits with status 101.
fn uncaught() {
    let _frame = Unwound("uncaught");
    leak(Payload::Message)
}

#[derive(Clone, Copy)]
enum Flavor {
    CurrentThread,
    MultiThread,
}

fn tokio_task(flavor: Flavor) {
    let runtime = match flavor {
        Flavor::CurrentThread => Builder::new_current_thread().enable_all().build(),
        Flavor::MultiThread => Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build(),
    }
    .unwrap();
    let caller = thread::current().id();
    let ran_on: Arc<Mutex<Option<ThreadId>>> = Arc::default();
    let recorded = Arc::clone(&ran_on);
    runtime.block_on(async move {
        let joined = tokio::spawn(async move {
            *recorded.lock().unwrap() = Some(thread::current().id());
            let _frame = Unwound("task");
            leak(Payload::Message);
        })
        .await;
        let error = joined.expect_err("the task panicked");
        assert!(error.is_panic());
        assert_payload(&*error.into_panic(), Payload::Message);
        // The runtime keeps scheduling tasks after the panic.
        assert_eq!(tokio::spawn(async { 7 }).await.unwrap(), 7);
    });
    let ran_on = ran_on.lock().unwrap().expect("the task ran");
    mark(if ran_on == caller {
        "task-on-caller-thread"
    } else {
        "task-on-worker-thread"
    });
    mark("payloads-intact");
}

fn std_thread() {
    let joined = thread::Builder::new()
        .name("named-worker".to_owned())
        .spawn(|| {
            let _frame = Unwound("thread");
            leak(Payload::Message)
        })
        .unwrap()
        .join();
    assert_payload(&*joined.expect_err("the thread panicked"), Payload::Message);
    mark("payloads-intact");
}

/// Another thread takes the `Stderr` lock before this one panics and releases
/// it only after the report is recorded, so a hook that took the lock would
/// block the panicking thread until the parent's guard killed the child.
fn stderr_lock_held() {
    let (locked, wait_for_lock) = mpsc::channel();
    let (release, wait_for_release) = mpsc::channel::<()>();
    let holder = thread::spawn(move || {
        let guard = std::io::stderr().lock();
        locked.send(()).unwrap();
        // Returns on release, or when a failed assertion drops the sender.
        let _ = wait_for_release.recv();
        drop(guard);
    });
    wait_for_lock.recv().unwrap();
    let unwound = catch_unwind(|| {
        let _frame = Unwound("lock-held");
        leak(Payload::Message)
    })
    .expect_err("the closure panicked");
    mark("reported-while-locked");
    release.send(()).unwrap();
    holder.join().unwrap();
    assert_payload(&*unwound, Payload::Message);
    mark("payloads-intact");
}

/// Exhaust descriptors under the parent's low limit, so duplicating standard
/// error fails: the panic still unwinds with its payload, and the hook writes
/// nothing rather than wait for the `Stderr` lock.
fn descriptor_limit() {
    let mut held = Vec::new();
    loop {
        match File::open("/dev/null") {
            Ok(file) => held.push(file),
            Err(error) if error.raw_os_error() == Some(TOO_MANY_OPEN_FILES) => break,
            Err(error) => panic!("unexpected open failure: {error}"),
        }
        assert!(held.len() < 4096, "the descriptor limit was not applied");
    }
    assert!(std::io::stderr().as_fd().try_clone_to_owned().is_err());
    mark("descriptors-exhausted");
    let unwound = catch_unwind(|| {
        let _frame = Unwound("descriptor-limit");
        leak(Payload::Message)
    })
    .expect_err("the closure panicked");
    drop(held);
    assert_payload(&*unwound, Payload::Message);
    mark("payloads-intact");
}

/// An application hook with a real heap address, so identity is observable,
/// and a count of the panics it saw.
struct Sentinel {
    address: *const (),
    seen: Arc<AtomicUsize>,
}

impl Sentinel {
    fn install() -> Self {
        let seen = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&seen);
        let hook: Box<dyn Fn(&PanicHookInfo<'_>) + Send + Sync> = Box::new(move |_| {
            counted.fetch_add(1, Ordering::SeqCst);
        });
        let address = ptr::from_ref(&*hook).cast::<()>();
        panic::set_hook(hook);
        Self { address, seen }
    }

    /// Whether the process hook is still this sentinel. Looking requires
    /// taking it, so it is reinstalled at once.
    fn is_installed(&self) -> bool {
        let hook = panic::take_hook();
        let same = ptr::addr_eq(ptr::from_ref(&*hook), self.address);
        panic::set_hook(hook);
        same
    }

    fn seen(&self) -> usize {
        self.seen.load(Ordering::SeqCst)
    }

    fn require_kept(&self, after: &str) {
        assert!(self.is_installed(), "the hook was replaced after {after}");
        mark(&format!("hook-kept:{after}"));
    }
}

/// Batter entrypoints that own process-wide signal handlers or catch panics
/// leave the application's hook installed, unwrapped, and route the panics
/// they catch through it exactly once.
fn entrypoints() {
    let sentinel = Sentinel::install();
    sentinel.require_kept("installed");
    drop(batter_core::redacting_panic_hook());
    sentinel.require_kept("constructor");
    let runtime = Builder::new_current_thread().enable_all().build().unwrap();
    runtime.block_on(async {
        let before = sentinel.seen();
        command_work_panics().await;
        assert_eq!(sentinel.seen(), before + 1);
        sentinel.require_kept("command");
        let before = sentinel.seen();
        startup_initializer_panics().await;
        assert_eq!(sentinel.seen(), before + 1);
        sentinel.require_kept("startup");
        let before = sentinel.seen();
        blocking_closure_panics().await;
        assert_eq!(sentinel.seen(), before + 1);
        sentinel.require_kept("run-blocking");
    });
    controls(&sentinel);
}

/// Each oracle detects the regression it exists for: identity sees a
/// wrapper, and the count sees a hook swapped out around a panic.
fn controls(sentinel: &Sentinel) {
    let before = sentinel.seen();
    let saved = panic::take_hook();
    panic::set_hook(Box::new(|_| {}));
    catch_unwind(|| panic!("swapped")).expect_err("panicked");
    panic::set_hook(saved);
    assert!(
        sentinel.is_installed(),
        "identity cannot see a restored swap"
    );
    assert_eq!(
        sentinel.seen(),
        before,
        "the swapped-out hook saw the panic"
    );
    mark("control:swap-counted");
    let inner = panic::take_hook();
    panic::set_hook(Box::new(move |info| inner(info)));
    assert!(!sentinel.is_installed(), "identity missed a wrapper");
    mark("control:wrapper-seen");
}

fn context() -> OperationContext {
    OperationOwner::new(BUDGET).unwrap().into_context()
}

fn cleanup_budget() -> CleanupBudget {
    CleanupBudget::new(BUDGET, BUDGET, Duration::from_secs(1)).unwrap()
}

fn command_work() -> Result<(), BoxError> {
    panic!("command work")
}

async fn command_work_panics() {
    let owner = Command::new(context(), cleanup_budget(), |_scope| {
        Box::pin(async { command_work() })
    })
    .start_with_unix_signals()
    .expect("command listeners install");
    let Err(CommandFailure::Report(report)) = owner.finish().await else {
        panic!("a panicking command did not fail with a report");
    };
    assert!(matches!(report.work, Err(CommandCause::Panicked(_))));
}

fn initializer() -> Result<(), ApplicationError> {
    panic!("initializer")
}

async fn startup_initializer_panics() {
    let second = Duration::from_secs(1);
    let budget = ShutdownBudget::new(second, second, second, cleanup_budget()).unwrap();
    let starting = Startup::protected(
        Supervisor::new(budget),
        context(),
        cleanup_budget(),
        async |_scope| initializer(),
    )
    .with_unix_signals("signals")
    .start();
    let Err(StartupError::Failed(failure)) = starting.finish_with_output().await else {
        panic!("a panicking initializer did not fail startup");
    };
    assert!(matches!(failure.cause, StartupCause::Panicked(_)));
}

fn blocking_work() {
    panic!("blocking closure")
}

async fn blocking_closure_panics() {
    let bulkhead = Bulkhead::new(BulkheadCapacity::new(1).unwrap());
    let outcome = bulkhead
        .run_blocking(&context(), Admission::Reject, blocking_work)
        .await;
    assert!(matches!(outcome, Err(BlockingError::Panicked(_))));
}
