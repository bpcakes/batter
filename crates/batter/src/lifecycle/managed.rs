//! Adapter-owned native runtimes with settlement retained outside their waiter.

mod drive;
mod report;

pub use report::{
    ManagedFailure, ManagedInitialization, ManagedObserver, ManagedOutcome, ManagedRecord,
    SettlementEvidence,
};

use super::{Component, ComponentFuture, ShutdownBudget};
use crate::{BoxError, operation::OperationContext};
use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};
use tokio::time::Instant;

type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send + 'static>>;

pub(super) async fn freeze(
    observers: Vec<(&'static str, ManagedObserver)>,
    handle: &super::ShutdownHandle,
    allowance: Duration,
) -> Vec<ManagedRecord> {
    let mut records = Vec::with_capacity(observers.len());
    for (name, observer) in observers {
        observer.until(handle.shared.phase_elapsed(allowance)).await;
        records.push(ManagedRecord {
            name,
            outcome: observer.snapshot(),
            observer,
        });
    }
    records
}

/// Native adapter evidence about all work owned by its runtime, including its
/// descendants. Implement on an adapter report wrapping the original native
/// report. Business failures need not prohibit releasing dependencies, whereas
/// unknown termination, aborted work and interrupted callbacks must do so.
pub trait ManagedSettlement: Send + Sync + 'static {
    /// Whether the complete native shutdown succeeded.
    fn is_success(&self) -> bool;

    /// Whether all native dependency users stopped cooperatively. Returning true
    /// is an adapter contract, not something Batter can infer from a wrapper join.
    fn allows_dependency_cleanup(&self) -> bool;
}

/// The portion of the supervisor's shutdown budget available to a native runtime.
/// Its abort observation consumes the parent's cancellation interval; the parent
/// retains its final abort/reap interval to observe and account for settlement.
#[derive(Clone, Copy, Debug)]
pub struct ManagedShutdownBudget {
    graceful: Duration,
    abort: Duration,
}

impl ManagedShutdownBudget {
    /// Maximum native graceful allowance, excluding scheduler delays.
    pub fn graceful(self) -> Duration {
        self.graceful
    }

    /// Maximum native abort/join allowance. Zero allows ready observations only.
    pub fn abort(self) -> Duration {
        self.abort
    }
}

/// Adapter-facing lifecycle parts. Application composition should use its native
/// adapter's registration function rather than implement this protocol itself.
///
/// `initialized` acknowledges actual local initialization; it must not perform a
/// durable startup witness. `stopping` observes native stop independently of full
/// settlement, so a native failure promptly drains other process components.
/// `stop` requests native shutdown synchronously using the supplied first-stop
/// timestamp and returns the earliest native/enclosing timestamp. Batter may call
/// it again to tighten a deadline when another component reveals an earlier stop.
/// It must be idempotent, may only shorten deadlines, and must retain the first
/// native cause independently of the deadline. Callback panics are retained in
/// the published outcome before awaiting settlement, including repeated calls.
/// `settlement` drives and reports
/// all native work within the supplied budget. All parts are polled/destroyed
/// under the owning tracing dispatch; default diagnostics omit their contents.
pub struct ManagedComponent<R> {
    initialized: BoxFuture<Result<(), BoxError>>,
    stopping: BoxFuture<()>,
    stop: Box<dyn FnMut(Instant) -> Instant + Send>,
    settlement: BoxFuture<R>,
}

impl<R: ManagedSettlement> ManagedComponent<R> {
    /// Transfer an initialized-observer, stop observer/control and complete
    /// native driver together, before returning from a registration factory.
    /// Do not spawn native work before that validated factory runs.
    ///
    /// This adapter-level example owns one actual native child. A multi-task
    /// runtime must supply its complete descendant report in the same place.
    ///
    /// ```
    /// use batter::{BoxError, cleanup::CleanupBudget, operation::OperationContext,
    ///     lifecycle::{ManagedComponent, ManagedSettlement, ShutdownBudget, Supervisor}};
    /// use std::{future::pending, time::Duration};
    /// struct NativeReport(Result<(), tokio::task::JoinError>);
    /// impl ManagedSettlement for NativeReport {
    ///     fn is_success(&self) -> bool { self.0.is_ok() }
    ///     fn allows_dependency_cleanup(&self) -> bool { self.0.is_ok() }
    /// }
    /// # #[tokio::main(flavor = "current_thread")]
    /// # async fn main() -> Result<(), BoxError> {
    /// let second = Duration::from_secs(1);
    /// let cleanup = CleanupBudget::new(second, second, second)?;
    /// let mut supervisor = Supervisor::new(ShutdownBudget::new(second, second, second, cleanup)?);
    /// supervisor.register_managed("native", OperationContext::new(second)?, |_| {
    ///     let (ack, initialized) = tokio::sync::oneshot::channel();
    ///     let (stop, stopping) = tokio::sync::oneshot::channel();
    ///     let mut stop = Some(stop);
    ///     let child = tokio::spawn(async move {
    ///         let _ = ack.send(()); // Local state is initialized inside its task.
    ///         let _ = stopping.await;
    ///     });
    ///     Ok(ManagedComponent::new(
    ///         async move { initialized.await.map_err(|error| Box::new(error) as BoxError) },
    ///         pending(), move |started| { if let Some(stop) = stop.take() { let _ = stop.send(()); } started },
    ///         async move { NativeReport(child.await) },
    ///     ))
    /// })?;
    /// let running = supervisor.start();
    /// running.handle().mark_ready(); // Explicit application approval.
    /// running.handle().wait_ready().await.unwrap();
    /// batter::lifecycle::check_shutdown(running.shutdown().await)?;
    /// # Ok(()) }
    /// ```
    pub fn new<I, N, S, F>(initialized: I, stopping: N, stop: S, settlement: F) -> Self
    where
        I: Future<Output = Result<(), BoxError>> + Send + 'static,
        N: Future<Output = ()> + Send + 'static,
        S: FnMut(Instant) -> Instant + Send + 'static,
        F: Future<Output = R> + Send + 'static,
    {
        Self {
            initialized: Box::pin(initialized),
            stopping: Box::pin(stopping),
            stop: Box::new(stop),
            settlement: Box::pin(settlement),
        }
    }

    fn erase(self) -> ManagedComponent<SettlementEvidence> {
        ManagedComponent {
            initialized: self.initialized,
            stopping: self.stopping,
            stop: self.stop,
            settlement: Box::pin(EraseSettlement {
                future: self.settlement,
            }),
        }
    }
}

// Do not implement this mapping with an async block consuming the inner future:
// dropping that await temporary could panic before its report is retained.
struct EraseSettlement<R> {
    future: BoxFuture<R>,
}

impl<R: ManagedSettlement> Future for EraseSettlement<R> {
    type Output = SettlementEvidence;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.future.as_mut().poll(cx).map(SettlementEvidence::new)
    }
}

pub(super) struct Registration {
    pub(super) name: &'static str,
    context: OperationContext,
    factory: Box<
        dyn FnOnce(ManagedShutdownBudget) -> Result<ManagedComponent<SettlementEvidence>, BoxError>
            + Send,
    >,
}

impl Registration {
    pub(super) fn new<F, R>(name: &'static str, context: OperationContext, factory: F) -> Self
    where
        F: FnOnce(ManagedShutdownBudget) -> Result<ManagedComponent<R>, BoxError> + Send + 'static,
        R: ManagedSettlement,
    {
        Self {
            name,
            context,
            factory: Box::new(move |budget| factory(budget).map(ManagedComponent::erase)),
        }
    }

    pub(super) fn prepare(self, budget: ShutdownBudget) -> (Component, ManagedObserver) {
        // Completion observation exists only once the supervisor driver starts.
        let (publication, completion) = tokio::sync::watch::channel(ManagedOutcome::default());
        let observer = ManagedObserver { completion };
        let waiter = observer.clone();
        let component = Component {
            name: self.name,
            factory: Box::new(move |signal| {
                Box::pin(async move {
                    drive::start(self, budget, signal, publication);
                    let outcome = waiter.wait().await;
                    if outcome.is_success() {
                        Ok(())
                    } else {
                        Err(Box::new(ManagedTaskFailed) as BoxError)
                    }
                }) as ComponentFuture
            }),
        };
        (component, observer)
    }
}

#[derive(Debug, thiserror::Error)]
#[error("managed component failed; inspect retained managed settlement")]
struct ManagedTaskFailed;
