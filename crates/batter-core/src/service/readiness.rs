use super::{Diagnostics, ServiceCompletion, ServiceObserver, ServiceOwner, driver};
use crate::{
    lifecycle::ReadinessProgress,
    scoped_dispatch::ScopedOutput,
    startup::{Initializer, ScopedStartup},
};
use std::fmt;
use tokio::{sync::oneshot, time::Instant};

/// Why this service never reached its first ready state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ServiceReadinessError {
    /// The absolute readiness deadline elapsed before the Ready transition.
    #[error("service readiness deadline exceeded")]
    DeadlineExceeded,
    /// Startup failed or drain/stop began before readiness.
    #[error("service stopped before becoming ready")]
    StoppedBeforeReady,
}

pub(super) fn readiness_result(progress: ReadinessProgress) -> Result<(), ServiceReadinessError> {
    match progress {
        ReadinessProgress::Ready => Ok(()),
        ReadinessProgress::DeadlineExceeded => Err(ServiceReadinessError::DeadlineExceeded),
        ReadinessProgress::Stopped | ReadinessProgress::Pending(_) => {
            Err(ServiceReadinessError::StoppedBeforeReady)
        }
    }
}

/// Start protected service execution with typed output and an optional absolute
/// readiness deadline. Diagnostics install before initialization. The deadline
/// covers initialization, driver startup, and every component acknowledgement;
/// optional dependency-health and application HTTP probe predicates are separate.
/// It is never restarted by a waiter. A past deadline prevents application work.
///
/// At or after the deadline the lifecycle cannot first become Ready. An owned
/// timer requests drain, and completion retains `DeadlineExceeded` separately
/// from startup, shutdown, and diagnostic results. A service ready before the
/// deadline is not drained by that timer. Cancellation is cooperative.
///
/// The sole output owner is returned immediately. Consume it with
/// [`StartingService::into_ready`] to receive output after readiness, or drop it
/// to request drain. Observers never acquire output or control authority.
///
/// ```no_run
/// use batter_core::{cleanup::CleanupBudget, lifecycle::{ShutdownBudget, Supervisor},
///     operation::OperationOwner, service::{self, NoDiagnostics}, startup::Startup};
/// use std::time::Duration;
/// # async fn example() -> Result<(), batter_core::BoxError> {
/// let second = Duration::from_secs(1);
/// let cleanup = CleanupBudget::new(second, second, second)?;
/// let process = Supervisor::new(ShutdownBudget::new(second, second, second, cleanup)?);
/// let deadline = tokio::time::Instant::now() + second;
/// let startup = Startup::protected(process, OperationOwner::new(second)?.into_context(),
///     cleanup, async |scope| {
///         scope.registration().register("worker", |startup| async {
///             let running = startup.acknowledge_started();
///             running.draining().await;
///             Ok(running.stopped())
///         })?;
///         Ok::<_, batter_core::RegistrationError>(42_u16)
///     });
/// let starting = service::start_with_readiness(startup, NoDiagnostics, Some(deadline));
/// let (owner, output) = starting.into_ready().await?;
/// assert_eq!(output, 42);
/// owner.request_shutdown();
/// let completion = owner.wait().await;
/// assert!(completion.readiness().is_ok());
/// # Ok(()) }
/// ```
pub fn start_with_readiness<F, E, D>(
    startup: ScopedStartup<F>,
    diagnostics: D,
    deadline: Option<Instant>,
) -> StartingService<E, D::Report, F::Output>
where
    F: Initializer<E>,
    E: Send + Sync + 'static,
    D: Diagnostics,
{
    driver::launch(startup, diagnostics, deadline)
}

/// Sole service and startup-output owner. Losing it requests drain; cancelling
/// a borrow of one retained `into_ready()` future leaves that future resumable.
/// No callback or cleanup runs in Drop, and runtime death cannot settle it.
#[must_use = "consume into_ready to claim startup output; dropping requests drain"]
pub struct StartingService<E, R, T> {
    pub(super) owner: ServiceOwner<E, R>,
    pub(super) output: oneshot::Receiver<ScopedOutput<T>>,
}

impl<E, R, T> StartingService<E, R, T> {
    /// Observe readiness and eventual completion without retaining ownership.
    pub fn observer(&self) -> ServiceObserver<E, R> {
        self.owner.observer()
    }

    /// Request drain while preserving the output owner and completion observer.
    pub fn request_shutdown(&self) {
        self.owner.request_shutdown();
    }

    /// Transfer the running owner and non-Clone output exactly once after first
    /// readiness. Readiness may subsequently be withdrawn; the returned owner's
    /// `status()` observes the current state. On failure, wait for full service
    /// and diagnostic settlement and retain both in the error.
    ///
    /// ```compile_fail,E0382
    /// # async fn once(starting: batter_core::service::StartingService<(), (), u16>) {
    /// let first = starting.into_ready().await;
    /// let second = starting.into_ready().await;
    /// # }
    /// ```
    pub async fn into_ready(self) -> Result<(ServiceOwner<E, R>, T), ServiceStartError<E, R>> {
        let Self { owner, output } = self;
        match output.await {
            Ok(value) => Ok((owner, value.into_inner())),
            Err(_) => Err(ServiceStartError {
                completion: owner.wait().await,
            }),
        }
    }
}

/// Startup output could not be delivered. Readiness, service, and diagnostic
/// outcomes remain independent; inspect all of them through `completion()`.
pub struct ServiceStartError<E, R> {
    completion: ServiceCompletion<E, R>,
}

impl<E, R> ServiceStartError<E, R> {
    /// Full retained completion, including any cleanup or diagnostic failure.
    pub fn completion(&self) -> &ServiceCompletion<E, R> {
        &self.completion
    }
}

impl<E, R> fmt::Debug for ServiceStartError<E, R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ServiceStartError")
    }
}

impl<E, R> fmt::Display for ServiceStartError<E, R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("service startup output unavailable; completion retained")
    }
}

impl<E: 'static, R: 'static> std::error::Error for ServiceStartError<E, R> {}
