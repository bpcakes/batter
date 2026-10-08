//! Library-owned recurring maintenance as a registered process component.
//!
//! Recurring pruning, storage cleanup, lease renewal and progress observation
//! register through [`register_periodic_in`]. Batter owns the serial schedule,
//! the per-run budget, startup acknowledgement, shutdown ordering and bounded
//! diagnostics. The application owns the operation itself and what it means
//! remotely.
//!
//! One registered component owns exactly one serial future: invocations never
//! overlap, nothing is spawned per run, and there is no second scheduler. The
//! first invocation is immediate; later ones follow a fixed interval with
//! missed ticks skipped.
//!
//! # Important limits
//!
//! Registration authorizes recurrence. It proves no remote rollback, no
//! idempotence, no successful renewal, no fencing and no absence of a
//! timed-out effect. Lease-loss and freshness policy, and durable witness
//! transitions, stay application- or native-owned. A deadline drops a run's
//! future; it does not undo whatever that run already did remotely. Timers
//! cannot preempt a factory, poll or destructor that blocks its runtime
//! thread, and a recoverable failure never triggers an automatic immediate
//! retry: the next scheduled invocation is the only continuation.
//!
//! ```no_run
//! use batter_core::{
//!     BoxError,
//!     operation::OperationContext,
//!     periodic::{PeriodicPolicy, PeriodicShutdown, PeriodicStartup},
//!     startup::ProtectedStartupScope,
//! };
//! use std::time::Duration;
//!
//! # async fn prune(_scope: OperationContext) -> Result<(), std::io::Error> { Ok(()) }
//! # fn register(scope: &mut ProtectedStartupScope) -> Result<(), BoxError> {
//! let policy = PeriodicPolicy::new(
//!     Duration::from_secs(30),
//!     Duration::from_secs(10),
//!     PeriodicStartup::immediate(),
//!     PeriodicShutdown::StopAtDrain,
//! )?;
//! let reader = batter_core::periodic::register_periodic_in(
//!     scope,
//!     "storage.pruning",
//!     policy,
//!     |scope| async move {
//!         // Replace with the actual bounded maintenance operation. `?` on an
//!         // ordinary application error keeps the schedule running.
//!         prune(scope).await?;
//!         Ok(())
//!     },
//! )?;
//! assert_eq!(reader.snapshot().invocations, 0);
//! # Ok(()) }
//! ```

mod driver;
mod failure;
mod history;
mod policy;

pub use failure::{PeriodicFailure, PeriodicInitializationExpired, PeriodicRun};
pub use history::{
    PeriodicCompletion, PeriodicFailureSample, PeriodicReader, PeriodicRecord, PeriodicSummary,
};
pub use policy::{PeriodicPolicy, PeriodicShutdown, PeriodicStartup};

pub(crate) use history::RetainedHistory;

use crate::{
    BoxError, RegistrationError,
    lifecycle::{ComponentExit, ComponentStartup, PeriodicAdmission, SupportObligation},
    operation::OperationContext,
    registration::RegistrationTarget,
};
use history::History;
use std::{future::Future, sync::Arc};

/// Register one serial periodic component through an existing registration
/// authority.
///
/// Registration is inert and validated: the name, interval, per-run budget and
/// startup allowance are checked before anything can run, and construction, a
/// rejected registration, an abandoned unstarted supervisor and a never-polled
/// factory all invoke no application work. The returned [`PeriodicReader`] is
/// read-only; reading it is never required, because the process completion
/// report already carries the same bounded evidence.
///
/// The work factory receives a library-created [`OperationContext`] whose
/// deadline is that run's own and whose cancellation descends from process
/// forced cancellation. Return [`PeriodicFailure::Fatal`] to initiate the
/// existing drain sequence; `?` on an ordinary application error yields a
/// retained recoverable failure and keeps the schedule.
///
/// ```
/// use batter_core::{
///     cleanup::CleanupBudget,
///     lifecycle::{ShutdownBudget, Supervisor},
///     periodic::{
///         PeriodicCompletion, PeriodicFailure, PeriodicPolicy, PeriodicShutdown, PeriodicStartup,
///     },
/// };
/// use std::{convert::Infallible, time::Duration};
///
/// # #[tokio::main(flavor = "current_thread", start_paused = true)]
/// # async fn main() -> Result<(), batter_core::BoxError> {
/// let second = Duration::from_secs(1);
/// let mut supervisor = Supervisor::new(ShutdownBudget::new(
///     second,
///     second,
///     second,
///     CleanupBudget::new(second, second, second)?,
/// )?);
/// let reader = batter_core::periodic::register_periodic_in(
///     &mut supervisor,
///     "storage.pruning",
///     PeriodicPolicy::new(
///         second,
///         second,
///         PeriodicStartup::immediate(),
///         PeriodicShutdown::StopAtDrain,
///     )?,
///     |_scope| async { Ok::<(), PeriodicFailure<Infallible>>(()) },
/// )?;
/// let running = supervisor.start();
/// running.status().wait_ready().await.unwrap();
/// let success = running.shutdown_checked().await?;
/// let summary = &success.report().periodic[0].summary;
/// assert_eq!(summary.completion, PeriodicCompletion::Stopped);
/// assert!(summary.succeeded >= 1);
/// assert_eq!(reader.snapshot().succeeded, summary.succeeded);
/// # Ok(()) }
/// ```
pub fn register_periodic_in<T, F, Fut, E>(
    target: &mut T,
    name: &'static str,
    policy: PeriodicPolicy,
    work: F,
) -> Result<PeriodicReader, RegistrationError>
where
    T: RegistrationTarget + ?Sized,
    F: FnMut(OperationContext) -> Fut + Send + 'static,
    Fut: Future<Output = Result<(), PeriodicFailure<E>>> + Send + 'static,
    E: std::error::Error + Send + Sync + 'static,
{
    target.registration().register_periodic(name, policy, work)
}

/// A validated periodic component awaiting its first live poll.
pub(crate) struct Registration<F> {
    name: &'static str,
    policy: PeriodicPolicy,
    work: F,
    history: Arc<History>,
}

impl<F> Registration<F> {
    pub(crate) fn new(name: &'static str, policy: PeriodicPolicy, work: F) -> Self {
        Self {
            name,
            policy,
            work,
            history: History::new(),
        }
    }

    /// Project read-only access for the registering application.
    pub(crate) fn reader(&self) -> PeriodicReader {
        self.history.reader(self.name)
    }

    /// Hand the supervisor its own retention of this component's evidence.
    pub(crate) fn retained(&self) -> RetainedHistory {
        RetainedHistory::new(self.name, self.history.clone())
    }

    /// Whether this component keeps running while other work drains.
    pub(crate) fn supports_through_drain(&self) -> bool {
        self.policy.shutdown() == PeriodicShutdown::SupportThroughDrain
    }

    /// Drive the component's one serial loop.
    pub(crate) fn run<Fut, E>(
        self,
        startup: ComponentStartup,
        admission: PeriodicAdmission,
        obligation: SupportObligation,
    ) -> impl Future<Output = Result<ComponentExit, BoxError>> + Send
    where
        F: FnMut(OperationContext) -> Fut + Send + 'static,
        Fut: Future<Output = Result<(), PeriodicFailure<E>>> + Send + 'static,
        E: std::error::Error + Send + Sync + 'static,
    {
        let Self {
            name,
            policy,
            work,
            history,
        } = self;
        driver::drive(name, policy, work, history, startup, admission, obligation)
    }
}
