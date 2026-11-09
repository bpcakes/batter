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
//! missed ticks skipped. The experimental [`register_wakeable_in`] also accepts
//! coalesced wake requests under a validated minimum start spacing.
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
//! retry: only the cadence or an explicit wake can request another invocation.
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
mod schedule;
mod wake;

pub(crate) use failure::PeriodicSuccess;
pub use failure::{PeriodicFailure, PeriodicInitializationExpired, PeriodicOutcome, PeriodicRun};
pub use history::{
    PeriodicCompletion, PeriodicFailureSample, PeriodicReader, PeriodicRecord, PeriodicSummary,
};
pub use policy::{PeriodicPolicy, PeriodicShutdown, PeriodicStartup};
pub use wake::{PeriodicWake, WakeablePolicy};

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
    target
        .registration()
        .register_periodic(Registration::new(name, policy, work))
}

/// Register an experimental wakeable serial periodic component.
///
/// This is the same protected execution path as [`register_periodic_in`], with
/// one coalesced wake request and a minimum start spacing. A cadence tick inside
/// that spacing window is deferred to its end. Wake runs never reset the
/// original cadence, and overdue ticks realign without a catch-up queue.
/// Pending wakes and due ticks coalesce into one invocation; requests arriving
/// during that invocation remain pending for the next eligible start.
///
/// Wakes cannot bypass startup allowance, admission, stopping or per-run budget.
/// Every admitted run counts in [`PeriodicSummary::invocations`]. No additional
/// task, cancellation protocol or join is required from the caller. Registration
/// is inert, and ending or dropping the component makes its wake handles inert.
///
/// ```
/// use batter_core::{
///     cleanup::CleanupBudget,
///     lifecycle::{ShutdownBudget, Supervisor},
///     periodic::{PeriodicPolicy, PeriodicStartup, PeriodicShutdown, WakeablePolicy,
///         PeriodicFailure, register_wakeable_in},
/// };
/// use std::{convert::Infallible, time::Duration};
/// # #[tokio::main(flavor = "current_thread", start_paused = true)]
/// # async fn main() -> Result<(), batter_core::BoxError> {
/// let second = Duration::from_secs(1);
/// let mut supervisor = Supervisor::new(ShutdownBudget::new(
///     second, second, second, CleanupBudget::new(second, second, second)?,
/// )?);
/// let policy = WakeablePolicy::new(PeriodicPolicy::new(
///     second * 30, second, PeriodicStartup::immediate(),
///     PeriodicShutdown::StopAtDrain,
/// )?, second)?;
/// assert_eq!(policy.wake_spacing(), second);
/// assert_eq!(policy.periodic().interval(), second * 30);
/// let (reader, wake) = register_wakeable_in(
///     &mut supervisor, "storage.refresh", policy,
///     |_| async { Ok::<(), PeriodicFailure<Infallible>>(()) },
/// )?;
/// let running = supervisor.start();
/// running.status().wait_ready().await.unwrap();
/// wake.clone().wake(); // A hint; the library owns spacing and execution.
/// tokio::time::sleep(second * 2).await;
/// assert_eq!(reader.snapshot().invocations, 2);
/// running.shutdown_checked().await?;
/// wake.wake(); // Inert after completion.
/// # Ok(()) }
/// ```
pub fn register_wakeable_in<T, F, Fut, E>(
    target: &mut T,
    name: &'static str,
    policy: WakeablePolicy,
    work: F,
) -> Result<(PeriodicReader, PeriodicWake), RegistrationError>
where
    T: RegistrationTarget + ?Sized,
    F: FnMut(OperationContext) -> Fut + Send + 'static,
    Fut: Future<Output = Result<(), PeriodicFailure<E>>> + Send + 'static,
    E: std::error::Error + Send + Sync + 'static,
{
    let (owner, wake) = wake::WakeOwner::new();
    let mut registration = Registration::new(name, policy.periodic(), work);
    registration.scheduling = schedule::Scheduling::Wakeable {
        owner,
        spacing: policy.wake_spacing(),
    };
    let reader = target.registration().register_periodic(registration)?;
    Ok((reader, wake))
}

/// Register a serial periodic component whose runs may decline their work.
///
/// The same protected execution path as [`register_periodic_in`], except that
/// each run's success is a [`PeriodicOutcome`]. A run that returns
/// `Ok(PeriodicOutcome::Skipped)`, such as one a tripped gate keeps out, is
/// admitted and counted in [`PeriodicSummary::invocations`] and
/// [`PeriodicSummary::skipped`], is not a success, and never acknowledges
/// startup. Skips are part of the completion report's periodic records.
///
/// ```
/// use batter_core::{
///     cleanup::CleanupBudget,
///     lifecycle::{ShutdownBudget, Supervisor},
///     periodic::{PeriodicFailure, PeriodicOutcome, PeriodicPolicy, PeriodicShutdown,
///         PeriodicStartup, register_skippable_in},
/// };
/// use std::{convert::Infallible, time::Duration};
/// # #[tokio::main(flavor = "current_thread", start_paused = true)]
/// # async fn main() -> Result<(), batter_core::BoxError> {
/// let second = Duration::from_secs(1);
/// let mut supervisor = Supervisor::new(ShutdownBudget::new(
///     second, second, second, CleanupBudget::new(second, second, second)?,
/// )?);
/// let policy = PeriodicPolicy::new(
///     second, second, PeriodicStartup::immediate(), PeriodicShutdown::StopAtDrain,
/// )?;
/// let reader = register_skippable_in(&mut supervisor, "storage.gated", policy, |_| async {
///     Ok::<_, PeriodicFailure<Infallible>>(PeriodicOutcome::Skipped)
/// })?;
/// let running = supervisor.start();
/// running.status().wait_ready().await.unwrap();
/// let success = running.shutdown_checked().await?;
/// let summary = &success.report().periodic[0].summary;
/// assert!(summary.skipped >= 1);
/// assert_eq!(summary.succeeded, 0);
/// assert_eq!(reader.snapshot().skipped, summary.skipped);
/// # Ok(()) }
/// ```
pub fn register_skippable_in<T, F, Fut, E>(
    target: &mut T,
    name: &'static str,
    policy: PeriodicPolicy,
    work: F,
) -> Result<PeriodicReader, RegistrationError>
where
    T: RegistrationTarget + ?Sized,
    F: FnMut(OperationContext) -> Fut + Send + 'static,
    Fut: Future<Output = Result<PeriodicOutcome, PeriodicFailure<E>>> + Send + 'static,
    E: std::error::Error + Send + Sync + 'static,
{
    target
        .registration()
        .register_periodic(Registration::new(name, policy, work))
}

/// A validated periodic component awaiting its first live poll.
pub(crate) struct Registration<F> {
    pub(crate) name: &'static str,
    policy: PeriodicPolicy,
    work: F,
    history: Arc<History>,
    scheduling: schedule::Scheduling,
}

impl<F> Registration<F> {
    pub(crate) fn new(name: &'static str, policy: PeriodicPolicy, work: F) -> Self {
        Self {
            name,
            policy,
            work,
            history: History::new(),
            scheduling: schedule::Scheduling::Cadence,
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
    pub(crate) fn run<Fut, S, E>(
        self,
        startup: ComponentStartup,
        admission: PeriodicAdmission,
        obligation: SupportObligation,
    ) -> impl Future<Output = Result<ComponentExit, BoxError>> + Send
    where
        F: FnMut(OperationContext) -> Fut + Send + 'static,
        Fut: Future<Output = Result<S, PeriodicFailure<E>>> + Send + 'static,
        S: crate::periodic::PeriodicSuccess,
        E: std::error::Error + Send + Sync + 'static,
    {
        driver::drive(self, startup, admission, obligation)
    }
}
