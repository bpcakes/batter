//! Kill-switch gates for registered periodic components.
//!
//! [`Flags::gate_periodic_in`] captures one view per scheduled run and admits
//! application work only on an open gate. Before the first publication and
//! while tripped, the run returns core's `PeriodicOutcome::Skipped` without
//! invoking the work factory. Core owns the serial schedule, run budgets,
//! failure retention, shutdown and the skip count, kept apart from successes.
//! A switch change reaches the next run; it does not cancel an admitted run.

use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

use batter_core::{
    ConfigurationError, RegistrationError,
    operation::OperationContext,
    periodic::{
        PeriodicFailure, PeriodicOutcome, PeriodicPolicy, PeriodicReader, PeriodicShutdown,
        PeriodicStartup, PeriodicSummary, register_skippable_in,
    },
    registration::RegistrationTarget,
};
use batter_flags_core::{
    FlagKey, KillSwitch, SnapshotView,
    low_level::{GateReading, HandleRefusal},
};

use crate::{Flags, flags::gate::SwitchGate};

/// Validated timing for gated maintenance: immediate startup, stop at drain.
///
/// Startup acknowledges only that the loop initialized. A skipped run must
/// never stand in for successful application initialization, so first-success
/// startup is not an option:
///
/// ```compile_fail,E0061
/// use batter_core::periodic::PeriodicStartup;
/// use batter_flags::periodic::GatedPeriodicPolicy;
/// use std::time::Duration;
/// let second = Duration::from_secs(1);
/// GatedPeriodicPolicy::new(second, second, PeriodicStartup::after_first_success(second).unwrap());
/// ```
///
/// Support work required for native settlement must register ungated through
/// [`register_periodic_in`](batter_core::periodic::register_periodic_in). A
/// tripped switch must not withhold that work
/// during drain, so support-through-drain is also excluded:
///
/// ```compile_fail,E0061
/// use batter_core::periodic::PeriodicShutdown;
/// use batter_flags::periodic::GatedPeriodicPolicy;
/// use std::time::Duration;
/// let second = Duration::from_secs(1);
/// GatedPeriodicPolicy::new(second, second, PeriodicShutdown::SupportThroughDrain);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GatedPeriodicPolicy {
    inner: PeriodicPolicy,
}

impl GatedPeriodicPolicy {
    /// Validate positive, representable timing using core's periodic limits.
    ///
    /// Runs never overlap; a run budget longer than the interval is valid.
    /// The first run is immediate and missed ticks are skipped.
    ///
    /// ```
    /// use batter_flags::periodic::GatedPeriodicPolicy;
    /// use std::time::Duration;
    /// let policy = GatedPeriodicPolicy::new(Duration::from_secs(30), Duration::from_secs(10))?;
    /// assert_eq!(policy.interval(), Duration::from_secs(30));
    /// assert_eq!(policy.run_budget(), Duration::from_secs(10));
    /// # Ok::<(), batter_core::ConfigurationError>(())
    /// ```
    pub fn new(interval: Duration, run_budget: Duration) -> Result<Self, ConfigurationError> {
        PeriodicPolicy::new(
            interval,
            run_budget,
            PeriodicStartup::immediate(),
            PeriodicShutdown::StopAtDrain,
        )
        .map(|inner| Self { inner })
    }

    /// Fixed interval between scheduled starts, with missed ticks skipped.
    pub fn interval(self) -> Duration {
        self.inner.interval()
    }

    /// Total budget for one admitted run, measured from its own start.
    pub fn run_budget(self) -> Duration {
        self.inner.run_budget()
    }
}

/// Why gated maintenance could not be registered.
///
/// Switch identity and ownership are checked before core registration, so a
/// refusal reserves no component name and invokes no application factory.
/// See [`Flags::gate_periodic_in`] for a complete registration example.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum GateRegistrationError {
    /// The switch belongs to another catalog.
    #[error("kill switch `{key}` belongs to another catalog")]
    ForeignHandle {
        /// Declared switch key, never an application value.
        key: FlagKey,
    },
    /// The flags' process kind does not own the switch.
    #[error("kill switch `{key}` is not owned by this process kind")]
    NotOwned {
        /// Declared switch key, never an application value.
        key: FlagKey,
    },
    /// Core refused the component registration, for example a duplicate name.
    #[error(transparent)]
    Registration(#[from] RegistrationError),
}

/// Read-only evidence for one gated component; owns no work or stop authority.
///
/// Cloning or dropping a reader does not affect execution. Skips are core's
/// own evidence, counted under the same lock as every other run and kept in
/// the completion report, so a skip is never counted as a success.
///
/// Migration: `snapshot().succeeded` counted skipped runs too, beside a count
/// these readers kept apart from core's; it now counts only runs whose work
/// succeeded, and `snapshot().skipped` holds the skips.
/// See [`Flags::gate_periodic_in`] for usage.
#[derive(Clone, Debug)]
pub struct GatedPeriodicReader {
    periodic: PeriodicReader,
}

impl GatedPeriodicReader {
    /// Validated name of this component.
    pub fn name(&self) -> &'static str {
        self.periodic.name()
    }

    /// Core's current retained evidence: `succeeded` counts runs whose work
    /// succeeded and `skipped` the runs the gate kept out.
    pub fn snapshot(&self) -> PeriodicSummary {
        self.periodic.snapshot()
    }

    /// Runs skipped while awaiting publication or tripped, saturating at
    /// `u64::MAX`; the same as `snapshot().skipped`.
    pub fn skipped(&self) -> u64 {
        self.periodic.snapshot().skipped
    }
}

impl Flags {
    /// Register gated maintenance with one library-captured view per run.
    ///
    /// Only an open gate invokes `work`, with the same frozen view used to
    /// admit it. Awaiting publication and tripped readings both skip, even
    /// when pre-publication reads otherwise serve defaults. Core counts skips
    /// in `PeriodicSummary::skipped` and the completion report, never as
    /// successes, so `succeeded` counts only runs whose work succeeded.
    /// A later switch change does not interrupt an active run. Core destroys
    /// that future on its deadline or drain, with no remote rollback claim.
    /// Ordinary errors remain recoverable; [`PeriodicFailure::Fatal`] drains.
    /// Registration, rejection and an unpolled driver do no application work.
    ///
    /// # Errors
    ///
    /// [`GateRegistrationError::ForeignHandle`] and
    /// [`GateRegistrationError::NotOwned`] precede core registration. Core's
    /// own refusals are retained in [`GateRegistrationError::Registration`].
    ///
    /// ```
    /// use batter_core::{
    ///     cleanup::CleanupBudget, lifecycle::{ShutdownBudget, Supervisor},
    ///     periodic::PeriodicFailure, settings::SettingsSource,
    /// };
    /// use batter_flags::{declared, core::{CatalogBuilder, CivilDate, SafeState, SettingsPrefix, StaleRule}, periodic::GatedPeriodicPolicy};
    /// use std::{convert::Infallible, time::Duration};
    /// # #[tokio::main(flavor = "current_thread", start_paused = true)]
    /// # async fn main() -> Result<(), batter_core::BoxError> {
    /// let (mut catalog, serving, hosted) = CatalogBuilder::new(SettingsPrefix::new("FLAG_")?, "serving", "hosted")?;
    /// let halt = catalog.kill_switch("prune_halt", SafeState::Engaged, StaleRule::HoldLastKnown).declare()?;
    /// let source = SettingsSource::from_pairs([])?;
    /// let flags = declared::resolve(catalog.build(), &source, serving, hosted, CivilDate::new(2026, 11, 6)?)?.into_flags();
    /// let second = Duration::from_secs(1);
    /// let mut supervisor = Supervisor::new(ShutdownBudget::new(
    ///     second, second, second, CleanupBudget::new(second, second, second)?,
    /// )?);
    /// let reader = flags.gate_periodic_in(
    ///     &mut supervisor, "storage.pruning", GatedPeriodicPolicy::new(second, second)?, &halt,
    ///     |_context, _view| async { Ok::<(), PeriodicFailure<Infallible>>(()) },
    /// )?;
    /// let running = supervisor.start();
    /// running.status().wait_ready().await.unwrap();
    /// let success = running.shutdown_checked().await?;
    /// assert_eq!(reader.name(), "storage.pruning");
    /// let summary = &success.report().periodic[0].summary;
    /// assert!(summary.skipped >= 1);
    /// assert_eq!(summary.succeeded, 0, "a skip is not a success");
    /// assert_eq!(reader.skipped(), summary.skipped);
    /// # Ok(()) }
    /// ```
    ///
    /// A native policy cannot bypass the gated policy's restrictions:
    ///
    /// ```compile_fail,E0308
    /// use batter_core::{lifecycle::Supervisor, periodic::{PeriodicPolicy, PeriodicFailure}};
    /// use batter_flags::{Flags, core::KillSwitch};
    /// fn register(flags: &Flags, target: &mut Supervisor, policy: PeriodicPolicy, halt: &KillSwitch) {
    ///     flags.gate_periodic_in(target, "storage.pruning", policy, halt,
    ///         |_, _| async { Ok::<(), PeriodicFailure<std::convert::Infallible>>(()) });
    /// }
    /// ```
    pub fn gate_periodic_in<T, F, Fut, E>(
        &self,
        target: &mut T,
        name: &'static str,
        policy: GatedPeriodicPolicy,
        switch: &KillSwitch,
        mut work: F,
    ) -> Result<GatedPeriodicReader, GateRegistrationError>
    where
        T: RegistrationTarget + ?Sized,
        F: FnMut(OperationContext, SnapshotView) -> Fut + Send + 'static,
        Fut: Future<Output = Result<(), PeriodicFailure<E>>> + Send + 'static,
        E: std::error::Error + Send + Sync + 'static,
    {
        let gate = SwitchGate::single(self, switch).map_err(|refusal| match refusal {
            HandleRefusal::ForeignCatalog => GateRegistrationError::ForeignHandle {
                key: switch.key().clone(),
            },
            HandleRefusal::NotOwned => GateRegistrationError::NotOwned {
                key: switch.key().clone(),
            },
        })?;
        let periodic = register_skippable_in(target, name, policy.inner, move |context| {
            let entered = gate.enter();
            match entered.reading {
                GateReading::AwaitingPublication(_) | GateReading::Tripped(_) => {
                    Invocation::Skipped
                }
                GateReading::Open(_) => Invocation::Work {
                    future: work(context, entered.view),
                },
            }
        })?;
        Ok(GatedPeriodicReader { periodic })
    }
}

pin_project_lite::pin_project! {
    /// Own the admitted future even after it returns Ready. Core retains its
    /// outcome before destroying this wrapper; awaiting the future by value
    /// here would let a destructor panic erase that result before retention.
    #[project = InvocationProjection]
    enum Invocation<F> {
        Skipped,
        Work { #[pin] future: F },
    }
}

impl<F, E> Future for Invocation<F>
where
    F: Future<Output = Result<(), PeriodicFailure<E>>>,
{
    type Output = Result<PeriodicOutcome, PeriodicFailure<E>>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.project() {
            InvocationProjection::Skipped => Poll::Ready(Ok(PeriodicOutcome::Skipped)),
            InvocationProjection::Work { future } => future
                .poll(cx)
                .map(|result| result.map(|()| PeriodicOutcome::Ran)),
        }
    }
}

#[cfg(test)]
mod tests;
