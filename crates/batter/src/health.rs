//! Owned dependency sampling with read-only, time-bounded observations.
//!
//! A [`HealthMonitor`] creates no tasks. [`HealthMonitor::register_in`] transfers
//! it to an ordinary supervised component and returns its read-only view. The
//! lower-level consuming [`HealthMonitor::run`] future remains available. Readers
//! never run probes. Dependency health is separate from lifecycle readiness:
//! adapters must check both.
//!
//! ```no_run
//! use batter::{health::{HealthMonitor, HealthPolicy, HealthReader},
//!     startup::ProtectedStartupScope};
//! use std::{convert::Infallible, time::Duration};
//! # fn register(scope: &mut ProtectedStartupScope)
//! #     -> Result<HealthReader<Infallible>, batter::BoxError> {
//! let policy = HealthPolicy::new(
//!     Duration::from_secs(1), Duration::from_secs(2),
//!     Duration::from_secs(4), Duration::from_secs(1),
//! )?;
//! Ok(HealthMonitor::new(policy, || async {
//!     // Replace with a native dependency probe, including its acquisition.
//!     Ok::<_, Infallible>(())
//! }).register_in(scope, "dependency.health")?)
//! # }
//! ```

mod observation;
mod policy;

pub use observation::{HealthReader, HealthSnapshot, HealthStatus, ProbeObservation, ProbeOutcome};
pub use policy::HealthPolicy;

use crate::{RegistrationError, lifecycle::ShutdownSignal, registration::RegistrationTarget};
use observation::Publication;
use std::{future::Future, sync::Arc};
use tokio::time::{Instant, sleep_until};

/// The sole dependency-health writer and native probe factory.
///
/// Construction is inert. Dropping this owner, including an unpolled `run`
/// future, immediately invalidates readers. It neither spawns nor joins hidden
/// work. Retained snapshots are historical values, not ownership of this writer.
/// E stays concrete and need not be Clone; shared failures use `Arc<E>`.
pub struct HealthMonitor<F, E> {
    policy: HealthPolicy,
    probe: F,
    publication: Arc<Publication<E>>,
}

impl<F, E> HealthMonitor<F, E> {
    /// Create a single writer without invoking the probe or spawning a task.
    /// The factory supplies the complete probe future, including resource
    /// acquisition. Do not detach work from that future.
    pub fn new<Fut>(policy: HealthPolicy, probe: F) -> Self
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<(), E>>,
    {
        Self {
            policy,
            probe,
            publication: Arc::new(Publication::new(policy.maximum_age())),
        }
    }

    /// Clone read-only access without prolonging writer ownership.
    pub fn reader(&self) -> HealthReader<E> {
        HealthReader::new(self.publication.clone())
    }

    /// Transfer this writer to a supervised component and return read-only access.
    ///
    /// Registration validates `name` before the component factory can run. If
    /// registration fails, the writer is dropped, no probe is invoked, and no
    /// reader is returned. The registered component acknowledges its own startup
    /// when [`Self::run`] is first polled and returns success after drain.
    ///
    /// ```
    /// use batter::{
    ///     health::{HealthMonitor, HealthPolicy, HealthReader},
    ///     startup::ProtectedStartupScope,
    /// };
    /// use std::{convert::Infallible, time::Duration};
    ///
    /// fn register(scope: &mut ProtectedStartupScope)
    ///     -> Result<HealthReader<Infallible>, batter::BoxError>
    /// {
    ///     let second = Duration::from_secs(1);
    ///     let policy = HealthPolicy::new(second, second, second * 3, second)?;
    ///     Ok(HealthMonitor::new(policy, || async { Ok::<_, Infallible>(()) })
    ///         .register_in(scope, "dependency.health")?)
    /// }
    /// ```
    pub fn register_in<T, Fut>(
        self,
        target: &mut T,
        name: &'static str,
    ) -> Result<HealthReader<E>, RegistrationError>
    where
        T: RegistrationTarget + ?Sized,
        F: FnMut() -> Fut + Send + 'static,
        Fut: Future<Output = Result<(), E>> + Send + 'static,
        E: Send + Sync + 'static,
    {
        let reader = self.reader();
        target
            .registration()
            .register(name, move |shutdown| async move {
                self.run(shutdown).await;
                Ok(())
            })?;
        Ok(reader)
    }

    /// Sample sequentially until drain or cancellation, then invalidate readers.
    ///
    /// Acknowledges monitor initialization on first poll; this does not claim a
    /// healthy dependency. Failures/timeouts publish unready observations and
    /// permit another attempt after the configured completion-to-next delay.
    /// Native panics propagate to the supervised task boundary and stop the writer.
    ///
    /// The whole supplied future is bounded, with drain/cancellation checked
    /// before deadlines and results. Active work is destroyed before publication
    /// or return. A late completion at/past the deadline is a timeout. Timers
    /// cannot preempt blocking factories, polls, or destructors, and dropping
    /// futures proves neither remote cancellation nor detached-task termination.
    /// No panic hook or global tracing subscriber is installed.
    pub async fn run<Fut>(self, shutdown: ShutdownSignal)
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<(), E>>,
    {
        crate::scoped_dispatch::scope(self.run_inner(shutdown)).await;
    }

    async fn run_inner<Fut>(mut self, shutdown: ShutdownSignal)
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<(), E>>,
    {
        if stopping(&shutdown) {
            return;
        }
        shutdown.mark_started();
        loop {
            if stopping(&shutdown) {
                return;
            }
            let Some(observation) = sample(&mut self.probe, &shutdown, self.policy).await else {
                return;
            };
            // sample has destroyed its owned future before this check. A drain
            // requested by that destruction cannot publish a later success.
            if stopping(&shutdown) {
                return;
            }
            self.publication.publish(observation);
            // A fresh delay after completion/destruction, never an interval's
            // catch-up schedule. Delayed polling cannot queue extra attempts.
            let next = Instant::now() + self.policy.probe_delay();
            tokio::select! {
                biased;
                _ = shutdown.draining() => return,
                _ = shutdown.cancelled() => return,
                _ = sleep_until(next) => {},
            }
        }
    }
}

impl<F, E> Drop for HealthMonitor<F, E> {
    fn drop(&mut self) {
        self.publication.stop();
    }
}

fn stopping(shutdown: &ShutdownSignal) -> bool {
    shutdown.is_draining() || shutdown.is_cancelled()
}

async fn sample<F, Fut, E>(
    probe: &mut F,
    shutdown: &ShutdownSignal,
    policy: HealthPolicy,
) -> Option<ProbeObservation<E>>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<(), E>>,
{
    let deadline = Instant::now() + policy.probe_budget();
    // Even factory invocation is inside the selected work branch: an already
    // observed drain/timeout invokes no application factory.
    let work = async { probe().await };
    tokio::pin!(work);
    tokio::select! {
        biased;
        _ = shutdown.draining() => None,
        _ = shutdown.cancelled() => None,
        _ = sleep_until(deadline) => Some(ProbeObservation::new(Instant::now(), ProbeOutcome::TimedOut)),
        result = &mut work => {
            let completed_at = Instant::now();
            let outcome = if completed_at >= deadline {
                ProbeOutcome::TimedOut
            } else {
                match result {
                    Ok(()) => ProbeOutcome::Succeeded,
                    Err(error) => ProbeOutcome::Failed(Arc::new(error)),
                }
            };
            Some(ProbeObservation::new(completed_at, outcome))
        },
    }
}
