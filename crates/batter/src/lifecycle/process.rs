use super::{ShutdownHandle, ShutdownSignal, TaskExit};
use crate::{ConfigurationError, RegistrationError, scoped_dispatch, validation};
use std::{
    error::Error,
    fmt,
    future::Future,
    pin::Pin,
    sync::{Arc, atomic::AtomicBool},
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, mpsc, oneshot};
use tracing::Instrument;

/// Admission is synchronous and bounded: no future or semaphore waiter is
/// created on rejection, and the application factory is never invoked.
#[derive(Debug, thiserror::Error)]
pub enum ProcessAdmissionError {
    /// The coordinator has not started and admission is not permanently closed.
    #[error("process driver has not started")]
    NotRunning,
    /// Application/component readiness has not been acknowledged.
    #[error("process is not ready")]
    NotReady,
    /// Root drain, forced cancellation, task failure, an expired parent scope,
    /// or a closed coordinator queue. Takes precedence over startup errors.
    #[error("process admission is closed")]
    Closed,
    /// Queued plus executing work already consumes the configured capacity.
    #[error("process work capacity is exhausted")]
    Full,
    /// Task names use the same static, sanitized vocabulary as components.
    #[error(transparent)]
    InvalidName(#[from] RegistrationError),
}

/// A finite task's typed result. A task-level failure initiates process shutdown;
/// put ordinary business rejection inside T (for example T = Result<Value, Denial>).
#[derive(Debug)]
pub enum ProcessTaskError<E: Error + 'static> {
    /// The original error is shared with the process shutdown report.
    /// `Error::source()` exposes E itself, retaining concrete downcasts.
    Failed(Arc<E>),
    /// Panic, abort, or coordinator cancellation prevented a typed result. The
    /// lifecycle report retains the observed JoinError when the driver completes.
    Terminated,
}

impl<E: Error + 'static> fmt::Display for ProcessTaskError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Failed(_) => "process task failed",
            Self::Terminated => "process task terminated without a typed result",
        })
    }
}

impl<E: Error + 'static> Error for ProcessTaskError<E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Failed(error) => Some(error.as_ref()),
            Self::Terminated => None,
        }
    }
}

/// Ownership of a result waiter only. Dropping this value or cancelling `wait`
/// does not cancel the admitted task or release its capacity permit.
pub struct ProcessReceipt<T, E: Error + 'static> {
    result: oneshot::Receiver<Result<T, Arc<E>>>,
}

impl<T, E: Error + 'static> ProcessReceipt<T, E> {
    /// Receive one typed result. The process continues to own the task even if
    /// this future is dropped while waiting.
    pub async fn wait(self) -> Result<T, ProcessTaskError<E>> {
        match self.result.await {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(error)) => Err(ProcessTaskError::Failed(error)),
            Err(_) => Err(ProcessTaskError::Terminated),
        }
    }
}

/// Cloneable bounded admission for finite process-owned tasks. This handle does
/// not keep an owned driver alive and cannot bypass shutdown admission policy.
#[derive(Clone)]
pub struct ProcessHandle {
    handle: ShutdownHandle,
    permits: Arc<Semaphore>,
    sender: mpsc::Sender<QueuedProcess>,
}

/// A capability belonging to one actively executing admitted task. Descendants
/// may be admitted during drain while this task is active, but never after
/// forced cancellation or a task-level failure. Every child consumes capacity.
/// Escaping/cloning the capability does not prolong the parent's active lifetime.
#[derive(Clone)]
pub struct ProcessScope {
    process: ProcessHandle,
    active: Arc<AtomicBool>,
}

impl ProcessScope {
    /// Read drain/forced-cancellation signals without authority over readiness.
    pub fn signal(&self) -> ShutdownSignal {
        self.process.handle.signal()
    }

    /// Submit a bounded descendant under this active ancestor. It has its own
    /// process lifetime; dropping the parent task's receipt cannot cancel it.
    pub fn try_spawn<F, Fut, T, E>(
        &self,
        name: &'static str,
        factory: F,
    ) -> Result<ProcessReceipt<T, E>, ProcessAdmissionError>
    where
        F: FnOnce(ProcessScope) -> Fut + Send + 'static,
        Fut: Future<Output = Result<T, E>> + Send + 'static,
        T: Send + 'static,
        E: Error + Send + Sync + 'static,
    {
        self.process.submit(name, factory, Some(&self.active))
    }
}

pub(super) struct QueuedProcess {
    pub name: &'static str,
    pub future: Pin<Box<dyn Future<Output = TaskExit> + Send + 'static>>,
}

impl ProcessHandle {
    pub(super) fn new(
        handle: ShutdownHandle,
        capacity: usize,
    ) -> Result<(Self, mpsc::Receiver<QueuedProcess>), ConfigurationError> {
        if capacity == 0 {
            return Err(ConfigurationError::Zero("process capacity"));
        }
        if capacity > Semaphore::MAX_PERMITS {
            return Err(ConfigurationError::TooLarge("process capacity"));
        }
        let (sender, receiver) = mpsc::channel(capacity);
        Ok((
            Self {
                handle,
                permits: Arc::new(Semaphore::new(capacity)),
                sender,
            },
            receiver,
        ))
    }

    /// Submit a finite task after process readiness. Capacity and drain share one
    /// linearization lock. The factory starts only inside a supervisor-owned task.
    ///
    /// An Err(E) is a process task failure and initiates shutdown. For a normal
    /// domain denial use Ok(Err(denial)) so the task itself completed successfully.
    ///
    /// ```
    /// use batter::lifecycle::{ShutdownCause, Supervisor};
    /// # use batter::{BoxError, cleanup::CleanupBudget, lifecycle::ShutdownBudget};
    /// # use std::time::Duration;
    /// # #[tokio::main(flavor = "current_thread")]
    /// # async fn main() -> Result<(), BoxError> {
    /// # let second = Duration::from_secs(1);
    /// # let budget = ShutdownBudget::new(
    /// #     second, second, second,
    /// #     CleanupBudget::new(second, second, second)?,
    /// # )?;
    /// let supervisor = Supervisor::with_process_capacity(budget, 1)?;
    /// let process = supervisor.process_handle().unwrap();
    /// supervisor.handle().mark_ready();
    /// let running = supervisor.start();
    /// running.handle().wait_ready().await.unwrap();
    /// let receipt = process.try_spawn("refresh", |_| async {
    ///     Err::<(), _>(std::io::Error::other("refresh failed"))
    /// })?;
    /// assert!(receipt.wait().await.is_err());
    /// let report = running.wait().await?;
    /// assert_eq!(report.cause, ShutdownCause::FiniteTaskExit("refresh"));
    /// # Ok(())
    /// # }
    /// ```
    pub fn try_spawn<F, Fut, T, E>(
        &self,
        name: &'static str,
        factory: F,
    ) -> Result<ProcessReceipt<T, E>, ProcessAdmissionError>
    where
        F: FnOnce(ProcessScope) -> Fut + Send + 'static,
        Fut: Future<Output = Result<T, E>> + Send + 'static,
        T: Send + 'static,
        E: Error + Send + Sync + 'static,
    {
        self.submit(name, factory, None)
    }

    fn submit<F, Fut, T, E>(
        &self,
        name: &'static str,
        factory: F,
        ancestor: Option<&Arc<AtomicBool>>,
    ) -> Result<ProcessReceipt<T, E>, ProcessAdmissionError>
    where
        F: FnOnce(ProcessScope) -> Fut + Send + 'static,
        Fut: Future<Output = Result<T, E>> + Send + 'static,
        T: Send + 'static,
        E: Error + Send + Sync + 'static,
    {
        validation::name(name)?;
        // Capture the submitting operation's causality before crossing the
        // queue. Span creation may invoke application subscriber code, so it
        // must happen outside the admission lock.
        let span = tracing::info_span!(target: "batter", "batter.process_task", task = name);
        let subscriber = tracing::dispatcher::get_default(Clone::clone);
        let mut admission = self.handle.shared.admission();
        admission.check(ancestor.map(Arc::as_ref), self.sender.is_closed())?;
        let permit = self
            .permits
            .clone()
            .try_acquire_owned()
            .map_err(|_| ProcessAdmissionError::Full)?;
        let active = Arc::new(AtomicBool::new(true));
        let scope = ProcessScope {
            process: self.clone(),
            active: active.clone(),
        };
        let (sender, result) = oneshot::channel();
        let handle = self.handle.clone();
        // This lease is constructed after enqueue succeeds below; no Drop that
        // re-locks admission may run while we still hold the admission lock.
        let (begin, begin_rx) = oneshot::channel::<ActiveTask>();
        let future = Box::pin(scoped_dispatch::with_dispatch(
            async move {
                let lease = begin_rx.await.expect("accepted process task has its lease");
                match factory(scope).await {
                    Ok(value) => {
                        // Finish the active scope before publishing its result so a
                        // waiter on another worker can immediately reuse capacity.
                        drop(lease);
                        let _ = sender.send(Ok(value));
                        TaskExit {
                            result: Ok(()),
                            expected: true,
                        }
                    }
                    Err(error) => {
                        handle.shared.fail_task();
                        let error = Arc::new(error);
                        drop(lease);
                        let _ = sender.send(Err(error.clone()));
                        TaskExit {
                            result: Err(Box::new(SharedError(error))),
                            expected: true,
                        }
                    }
                }
            }
            .instrument(span),
            subscriber,
        ));
        if let Err(error) = self.sender.try_send(QueuedProcess { name, future }) {
            // Rejected work owns arbitrary application captures. Their native
            // destructors may request shutdown, so never drop them under our
            // admission lock.
            drop(admission);
            return Err(match error {
                mpsc::error::TrySendError::Full(_) => ProcessAdmissionError::Full,
                mpsc::error::TrySendError::Closed(_) => ProcessAdmissionError::Closed,
            });
        }
        admission.admit_finite();
        drop(admission);
        let lease = ActiveTask {
            handle: self.handle.clone(),
            active,
            _permit: permit,
        };
        // The task is already owned, including if the coordinator aborts before
        // its first poll. In that case send drops the lease and releases capacity.
        let _ = begin.send(lease);
        Ok(ProcessReceipt { result })
    }
}

struct ActiveTask {
    handle: ShutdownHandle,
    active: Arc<AtomicBool>,
    _permit: OwnedSemaphorePermit,
}

impl Drop for ActiveTask {
    fn drop(&mut self) {
        self.handle.shared.finish_finite(&self.active);
    }
}

struct SharedError<E>(Arc<E>);
impl<E> fmt::Debug for SharedError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SharedProcessError")
    }
}
impl<E> fmt::Display for SharedError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("process task failed")
    }
}
impl<E: Error + 'static> Error for SharedError<E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.0.as_ref())
    }
}
