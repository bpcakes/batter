//! Constrained registration authority for owned startup and native adapters.
//!
//! [`Registration`] can add supervised work and reserve finalization, but it
//! cannot start, replace, or extract the enclosing supervisor. Applications use
//! [`crate::startup::ProtectedStartupScope`] as the canonical startup target;
//! direct [`crate::lifecycle::Supervisor`] support preserves adapter composition.

use crate::{
    BoxError, RegistrationError,
    cleanup::CleanupSlot,
    lifecycle::{
        ManagedComponent, ManagedSettlement, ManagedShutdownBudget, ShutdownSignal, Supervisor,
    },
    operation::OperationContext,
};
use std::future::Future;

/// A borrowed, registration-only view of an existing supervisor.
///
/// The view has no public constructor and exposes no conversion, dereference, or
/// access to the enclosing supervisor. Reborrow it with [`Self::registration`]
/// when several adapter helpers are called in sequence.
///
/// ```compile_fail,E0521
/// # fn cannot_escape(scope: &mut batter::startup::ProtectedStartupScope) {
/// let registration = scope.registration();
/// tokio::spawn(async move { drop(registration) });
/// # }
/// ```
pub struct Registration<'a> {
    supervisor: &'a mut Supervisor,
}

impl<'a> Registration<'a> {
    pub(crate) fn new(supervisor: &'a mut Supervisor) -> Self {
        Self { supervisor }
    }

    /// Register a long-lived critical component through the supervisor's
    /// existing validated and inert registration path.
    pub fn register<F, Fut>(
        &mut self,
        name: &'static str,
        factory: F,
    ) -> Result<(), RegistrationError>
    where
        F: FnOnce(ShutdownSignal) -> Fut + Send + 'static,
        Fut: Future<Output = Result<(), BoxError>> + Send + 'static,
    {
        self.supervisor.register(name, factory)
    }

    /// Register an owned native runtime without exposing process control.
    /// Validation precedes invocation of the managed factory, exactly as for
    /// [`Supervisor::register_managed`].
    pub fn register_managed<F, R>(
        &mut self,
        name: &'static str,
        context: OperationContext,
        factory: F,
    ) -> Result<(), RegistrationError>
    where
        F: FnOnce(ManagedShutdownBudget) -> Result<ManagedComponent<R>, BoxError> + Send + 'static,
        R: ManagedSettlement,
    {
        self.supervisor.register_managed(name, context, factory)
    }

    /// Reserve a validated finalizer name before acquiring a dependency.
    /// The returned slot borrows the actual supervisor cleanup stack and can be
    /// held across the native acquisition await.
    pub fn reserve_cleanup(
        &mut self,
        name: &'static str,
    ) -> Result<CleanupSlot<'_>, RegistrationError> {
        self.supervisor.reserve_cleanup(name)
    }

    /// Reborrow this authority for another helper call.
    pub fn registration(&mut self) -> Registration<'_> {
        Registration::new(self.supervisor)
    }
}

pub(crate) mod private {
    pub trait Sealed {}
}

/// A sealed target accepted by Batter's native adapter registration helpers.
///
/// Applications cannot implement this trait. Supported targets are
/// [`Supervisor`], [`Registration`], and
/// [`crate::startup::ProtectedStartupScope`]. The returned view never grants
/// process start, cleanup extraction, or supervisor replacement authority.
///
/// ```compile_fail,E0277
/// struct Forged;
/// impl batter::registration::RegistrationTarget for Forged {
///     fn registration(&mut self) -> batter::registration::Registration<'_> {
///         unreachable!()
///     }
/// }
/// ```
pub trait RegistrationTarget: private::Sealed {
    /// Borrow only the target's registration authority.
    #[doc(hidden)]
    fn registration(&mut self) -> Registration<'_>;
}

impl private::Sealed for Supervisor {}
impl RegistrationTarget for Supervisor {
    fn registration(&mut self) -> Registration<'_> {
        Registration::new(self)
    }
}

impl private::Sealed for Registration<'_> {}
impl RegistrationTarget for Registration<'_> {
    fn registration(&mut self) -> Registration<'_> {
        Registration::new(self.supervisor)
    }
}
