//! External stop inputs live in tracked tasks, never in the settlement owner.

use std::{
    error::Error,
    fmt,
    future::Future,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use futures_util::future::BoxFuture;

use crate::shutdown::ShutdownSignal;

mod owner;

/// Reserved identity shared by signal admission, cause publication and reports.
pub(crate) const TASK_NAME: &str = "shutdown_signal";

/// An owned, supervised input that requests runtime shutdown.
///
/// Use [`Self::unix_termination`] for SIGINT/SIGTERM, [`Self::ctrl_c`] for
/// Ctrl-C alone, [`Self::fallible`] for a signal that can
/// fail, or [`Self::infallible`] for a notification with no error result.
/// Signal errors are retained in the shutdown report. Completion or a caught
/// polling panic publishes stop as soon as it is observed, before guarded
/// destruction. Its destruction and join remain tracked settlement obligations. Polling and
/// destruction panics are retained as joins of the `shutdown_signal` descendant;
/// neither unwinds the native settlement owner. When both operations panic,
/// [`crate::RuntimeShutdownReport::signal_panic`] retains both observations.
/// Dropping an unsubmitted signal also contains destruction panic and emits a
/// redacted diagnostic; no report exists before submission.
///
/// Custom futures must be cancellation-safe: another stop source may win and
/// drop the signal. They must not leave detached application work using runtime
/// dependencies. If the budget force-aborts a custom signal, its cancelled join
/// is retained and dependency cleanup is denied. Only the library-authored
/// [`Self::unix_termination`], [`Self::ctrl_c`] and [`Self::pending`] listeners account for owner
/// cancellation after their guarded destruction joins. Like any Tokio task, a
/// signal must yield; a blocking poll or destructor cannot be forcibly stopped
/// by an async shutdown budget.
/// On Unix, [`Self::ctrl_c`] also requires signal support on the captured Tokio
/// runtime. Custom runtime builders must call
/// [`tokio::runtime::Builder::enable_io`] or
/// [`tokio::runtime::Builder::enable_all`]. If that prerequisite is absent,
/// Tokio panics while registering the listener; Runledger contains the panic as
/// signal-panic and descendant-join evidence rather than unwinding settlement.
/// Containment requires unwinding to reach this owner's boundary: it cannot
/// recover from `panic = "abort"`, an aborting panic hook, or a double panic
/// entirely inside application code before its poll or destructor unwinds.
/// `catch_unwind` does not suppress the process-global panic hook; Runledger
/// redacts its own diagnostics, while hook policy belongs to the embedding
/// process.
///
/// A fallible future cannot accidentally be declared infallible:
///
/// ```compile_fail
/// use runledger_runtime::RuntimeShutdownSignal;
/// let signal = RuntimeShutdownSignal::infallible(tokio::signal::ctrl_c());
/// ```
#[must_use = "pass the signal to Supervisor::run_until_shutdown_report"]
pub struct RuntimeShutdownSignal {
    task: ShutdownSignalTaskFactory,
}

type SignalTaskFactory =
    Box<dyn FnOnce(ShutdownSignal, Arc<ShutdownSignalTaskState>) -> BoxFuture<'static, ()> + Send>;

/// Keep the source coupled to the factory until task admission. Only built-ins
/// have cancellation behavior known by this crate; application futures remain
/// conservatively classified if the settlement owner has to abort them.
enum ShutdownSignalTaskFactory {
    RuntimeAuthored(SignalTaskFactory),
    ApplicationSupplied(SignalTaskFactory),
}

/// A prepared signal whose provenance determines cancellation classification.
/// The registry exhaustively consumes this enum, so callers cannot attach a
/// trusted policy to an application-supplied future as a separate choice.
pub(crate) enum ShutdownSignalTask {
    RuntimeAuthored {
        future: BoxFuture<'static, ()>,
        state: Arc<ShutdownSignalTaskState>,
    },
    ApplicationSupplied {
        future: BoxFuture<'static, ()>,
        state: Arc<ShutdownSignalTaskState>,
    },
}

/// Shared only by a signal task, the shutdown arbiter, and its registry entry.
/// The arbiter marks it while winning first-cause publication, so no caller can
/// manufacture initiating state independently of the signal that produced it.
pub(crate) struct ShutdownSignalTaskState {
    initiating: AtomicBool,
}

/// Every terminal poll observation is a stop trigger, including a caught panic.
/// Destruction and joining remain separate settlement obligations.
pub(crate) enum SignalTrigger {
    Output(Option<RuntimeShutdownSignalError>),
    PollPanicked {
        id: tokio::task::Id,
        message: String,
    },
}

impl ShutdownSignalTaskState {
    fn new() -> Self {
        Self {
            initiating: AtomicBool::new(false),
        }
    }

    pub(crate) fn mark_initiating(&self) {
        self.initiating.store(true, Ordering::Release);
    }

    pub(crate) fn is_initiating(&self) -> bool {
        self.initiating.load(Ordering::Acquire)
    }

    #[cfg(test)]
    pub(crate) fn initiating_for_tests() -> Self {
        let state = Self::new();
        state.mark_initiating();
        state
    }

    #[cfg(test)]
    pub(crate) fn pending_for_tests() -> Self {
        Self::new()
    }
}

impl RuntimeShutdownSignal {
    /// Install SIGINT and SIGTERM listeners on the current Tokio runtime.
    ///
    /// Call this at the executable root before starting application work. Both
    /// listeners are installed before this returns; a signal arriving before
    /// submission remains observable by the supervised signal future. Setup
    /// errors are returned to the root, never replaced by an inert listener.
    /// The returned signal has runtime-owned cancellation accounting, like
    /// [`Self::ctrl_c`]. Dropping it does not restore OS default dispositions:
    /// Tokio's process-wide signal handlers remain installed.
    ///
    /// # Errors
    /// Returns the native I/O error if either listener cannot be installed.
    ///
    /// # Panics
    /// Panics outside a Tokio runtime with I/O enabled. Keep that runtime alive
    /// and driven through shutdown, including while waiting on another runtime.
    ///
    /// ```no_run
    /// # async fn example(pool: &runledger_postgres::DbPool) -> Result<(), Box<dyn std::error::Error>> {
    /// let signal = runledger_runtime::RuntimeShutdownSignal::unix_termination()?;
    /// let budget = runledger_runtime::RuntimeShutdownBudget::new(
    ///     std::time::Duration::from_secs(30), std::time::Duration::from_secs(5),
    /// )?;
    /// let supervisor = runledger_runtime::Supervisor::builder_from_env(pool)?.build()?;
    /// let report = supervisor.run_until_shutdown_report(signal, budget).await;
    /// match report.classify() {
    ///     runledger_runtime::RuntimeSettlement::Clean(clean) => { let _permit = clean.into_cleanup_permit(); }
    ///     runledger_runtime::RuntimeSettlement::StoppedWithFailures(stopped) => { let (_permit, _failure) = stopped.into_parts(); }
    ///     runledger_runtime::RuntimeSettlement::Unsettled(unsettled) => { let _failure = unsettled.into_failure(); }
    /// }
    /// # Ok(()) }
    /// ```
    pub fn unix_termination() -> std::io::Result<Self> {
        use tokio::signal::unix::{SignalKind, signal};
        let mut interrupt = signal(SignalKind::interrupt())?;
        let mut terminate = signal(SignalKind::terminate())?;
        Ok(Self::from_runtime_future(
            async move {
                tokio::select! {
                    received = interrupt.recv() => received,
                    received = terminate.recv() => received,
                }
                .ok_or_else(|| std::io::Error::other("Unix termination listener closed"))
            },
            |result| result.err().map(RuntimeShutdownSignalError::new),
        ))
    }

    /// Wait for Ctrl-C, preserving signal-handler setup errors in the report.
    /// The handler is installed on the runtime captured by the supervisor.
    /// On Unix, a custom captured runtime must enable I/O (or all drivers) so
    /// Tokio's signal driver is available. Missing signal-driver support is
    /// retained as signal-panic and descendant-join evidence.
    pub fn ctrl_c() -> Self {
        Self::from_runtime_future(tokio::signal::ctrl_c(), |result| {
            result.err().map(RuntimeShutdownSignalError::new)
        })
    }

    /// Wait only for another shutdown source, such as a supervisor handle.
    pub fn pending() -> Self {
        Self::from_runtime_future(std::future::pending(), |()| None)
    }

    /// Supervise a fallible signal without discarding its error or panicking.
    pub fn fallible<F, E>(future: F) -> Self
    where
        F: Future<Output = Result<(), E>> + Send + 'static,
        E: Error + Send + Sync + 'static,
    {
        Self::from_application_future(future, |result| {
            result.err().map(RuntimeShutdownSignalError::new)
        })
    }

    /// Supervise a notification whose output cannot report failure.
    /// Do not use this to erase errors from a fallible signal.
    pub fn infallible<F>(future: F) -> Self
    where
        F: Future<Output = ()> + Send + 'static,
    {
        Self::from_application_future(future, |()| None)
    }

    fn from_runtime_future<F, T>(
        future: F,
        error: impl FnOnce(T) -> Option<RuntimeShutdownSignalError> + Send + 'static,
    ) -> Self
    where
        F: Future<Output = T> + Send + 'static,
    {
        Self {
            task: ShutdownSignalTaskFactory::RuntimeAuthored(Self::task_factory(future, error)),
        }
    }

    fn from_application_future<F, T>(
        future: F,
        error: impl FnOnce(T) -> Option<RuntimeShutdownSignalError> + Send + 'static,
    ) -> Self
    where
        F: Future<Output = T> + Send + 'static,
    {
        Self {
            task: ShutdownSignalTaskFactory::ApplicationSupplied(Self::task_factory(future, error)),
        }
    }

    fn task_factory<F, T>(
        future: F,
        error: impl FnOnce(T) -> Option<RuntimeShutdownSignalError> + Send + 'static,
    ) -> SignalTaskFactory
    where
        F: Future<Output = T> + Send + 'static,
    {
        // Acquire the application future once, before either the factory or the
        // spawned task can be dropped. No later path owns an unguarded F.
        let future = owner::SignalFuture::new(future);
        Box::new(move |shutdown, state| {
            let future = future.supervise(shutdown.clone(), state, error);
            Box::pin(async move {
                let mut future = Box::pin(future);
                tokio::select! {
                    biased;
                    () = shutdown.requested() => {},
                    () = future.as_mut() => {},
                }
                drop(future);
            })
        })
    }

    pub(crate) fn into_task(self, shutdown: ShutdownSignal) -> ShutdownSignalTask {
        let state = Arc::new(ShutdownSignalTaskState::new());
        match self.task {
            ShutdownSignalTaskFactory::RuntimeAuthored(task) => {
                ShutdownSignalTask::RuntimeAuthored {
                    future: task(shutdown, Arc::clone(&state)),
                    state,
                }
            }
            ShutdownSignalTaskFactory::ApplicationSupplied(task) => {
                ShutdownSignalTask::ApplicationSupplied {
                    future: task(shutdown, Arc::clone(&state)),
                    state,
                }
            }
        }
    }
}

/// Panic observations from one signal. Every variant contains an observed
/// failure; absence of a panic is represented by `None` on the report.
/// Formatting redacts message contents. Explicit field access exposes them.
#[derive(Clone)]
pub enum RuntimeShutdownSignalPanic {
    Poll {
        message: String,
    },
    Destruction {
        message: String,
    },
    PollAndDestruction {
        poll_message: String,
        destruction_message: String,
    },
}

impl fmt::Debug for RuntimeShutdownSignalPanic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Poll { .. } => "Poll",
            Self::Destruction { .. } => "Destruction",
            Self::PollAndDestruction { .. } => "PollAndDestruction",
        })
    }
}

/// A returned signal error. Formatting redacts application-provided contents;
/// use [`Error::source`] to explicitly inspect the original typed error.
#[derive(Clone)]
pub struct RuntimeShutdownSignalError(Arc<dyn Error + Send + Sync>);

impl RuntimeShutdownSignalError {
    pub(crate) fn new(error: impl Error + Send + Sync + 'static) -> Self {
        Self(Arc::new(error))
    }
}

impl fmt::Display for RuntimeShutdownSignalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("shutdown signal failed")
    }
}

impl fmt::Debug for RuntimeShutdownSignalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RuntimeShutdownSignalError")
    }
}

impl Error for RuntimeShutdownSignalError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.0.as_ref())
    }
}
