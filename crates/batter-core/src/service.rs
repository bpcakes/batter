//! Protected startup through shutdown and independent diagnostic finalization.
//!
//! [`start`] consumes an inert [`crate::startup::ScopedStartup`], installs explicitly supplied
//! diagnostics, and owns both until completion. It accepts no arbitrary service
//! future: only the protected startup/driver path can release diagnostics for
//! finalization. Owner loss requests drain; borrowed wait cancellation does not.
//! Diagnostic panics retain their payload separately from the original service
//! result. The runtime must outlive all phases; synchronous or non-yielding
//! diagnostic code cannot be preempted by an asynchronous deadline.
//!
//! ```no_run
//! use batter_core::{cleanup::CleanupBudget, lifecycle::{ShutdownBudget, Supervisor},
//!     operation::OperationOwner, service::{self, NoDiagnostics}, startup::Startup};
//! use std::time::Duration;
//! # async fn example() -> Result<(), batter_core::BoxError> {
//! let second = Duration::from_secs(1);
//! let cleanup = CleanupBudget::new(second, second, second)?;
//! let supervisor = Supervisor::new(ShutdownBudget::new(second, second, second, cleanup)?);
//! let startup = Startup::scoped(supervisor, OperationOwner::new(second)?.into_context(),
//!     cleanup, |_| Box::pin(async { Ok::<_, std::io::Error>(()) }));
//! let owner = service::start(startup, NoDiagnostics);
//! let observer = owner.observer();
//! drop(owner); // Request drain; the independent coordinator retains cleanup.
//! let completion = observer.wait().await;
//! let _service = completion.service();
//! let _diagnostics = completion.diagnostics();
//! # Ok(()) }
//! ```

mod coverage;
mod driver;
mod report;

pub use coverage::{CompletionCoverage, IncompleteCoverage, RetainedCompletion};
pub use driver::{ServiceObserver, ServiceOwner, start};
pub use report::{DiagnosticOutcome, ServiceCompletion, ServiceOutcome};

use crate::completion::wait_published;
use tokio::sync::watch;

/// Lower-level extension seam for supported diagnostic adapters.
///
/// Installation must be synchronous and inert with respect to application work.
/// The returned future owns periodic work and must await `completion` before
/// collecting its final snapshot or closing resources. It must bound its own
/// I/O and retain transport failures in `Report`. Implementations may run before
/// startup's first poll and must not drive or cancel the service themselves.
/// These obligations belong to adapter implementations; use a supported adapter
/// for the protected consumer path. A custom implementation can violate them.
/// Installation and future panics are isolated, but blocking/non-yielding code,
/// aborting panics and runtime death cannot be shielded.
pub trait Diagnostics: Send + 'static {
    /// Adapter-owned report, retained separately from the service result.
    type Report: Send + Sync + 'static;

    /// Install once, returning the independently driven diagnostic future.
    fn install(
        self,
        completion: DiagnosticCompletion,
    ) -> impl Future<Output = Self::Report> + Send + 'static;
}

/// Opaque observation supplied only by [`start`] to a diagnostic adapter.
/// This carries no shutdown authority and cannot be constructed by consumers.
///
/// ```compile_fail
/// use batter_core::service::DiagnosticCompletion;
/// let completion = DiagnosticCompletion::default();
/// ```
pub struct DiagnosticCompletion(watch::Receiver<Option<CompletionCoverage>>);

impl DiagnosticCompletion {
    /// Wait for retained startup failure cleanup or running-driver completion.
    /// The witness describes report coverage, not global process quiescence.
    /// Panics if runtime destruction prevents publication. Cancelling this wait
    /// does not cancel service execution or cleanup.
    pub async fn wait(self) -> RetainedCompletion {
        RetainedCompletion(
            wait_published(self.0)
                .await
                .expect("service monitor retains publication"),
        )
    }
}

/// Explicit selection of no diagnostic installation or work.
pub struct NoDiagnostics;

impl Diagnostics for NoDiagnostics {
    type Report = ();

    async fn install(self, completion: DiagnosticCompletion) {
        let _ = completion.wait().await;
    }
}
