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
        ComponentExit, ComponentStartup, ManagedComponent, ManagedSettlement,
        ManagedShutdownBudget, Supervisor,
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
/// # fn cannot_escape(scope: &mut batter_core::startup::ProtectedStartupScope) {
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
        F: FnOnce(ComponentStartup) -> Fut + Send + 'static,
        Fut: Future<Output = Result<ComponentExit, BoxError>> + Send + 'static,
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

    /// Register one library-owned serial periodic component through the
    /// supervisor's validated and inert registration path. See
    /// [`crate::periodic::register_periodic_in`] for the public entry point.
    pub(crate) fn register_periodic<F, Fut, S, E>(
        &mut self,
        registration: crate::periodic::Registration<F>,
    ) -> Result<crate::periodic::PeriodicReader, RegistrationError>
    where
        F: FnMut(OperationContext) -> Fut + Send + 'static,
        Fut: Future<Output = Result<S, crate::periodic::PeriodicFailure<E>>> + Send + 'static,
        S: crate::periodic::PeriodicSuccess,
        E: std::error::Error + Send + Sync + 'static,
    {
        self.supervisor.register_periodic(registration)
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

    /// Refuses, before anything is registered, every component name in
    /// `components` and finalizer name in `cleanups` that is invalid, already
    /// registered or reserved, or listed twice.
    ///
    /// A helper that registers several components and finalizers checks them
    /// all first, while it holds this exclusive authority, so its registration
    /// either installs every piece or, refused here, none: nothing else can
    /// register between the check and the registrations.
    ///
    /// ```
    /// # use batter_core::{cleanup::CleanupBudget, lifecycle::{ShutdownBudget, Supervisor}, registration::RegistrationTarget, RegistrationError};
    /// # let second = std::time::Duration::from_secs(1);
    /// # let cleanup = CleanupBudget::new(second, second, second).unwrap();
    /// # let mut owner = Supervisor::new(ShutdownBudget::new(second, second, second, cleanup).unwrap());
    /// let registration = owner.registration();
    /// assert!(registration.check_names(&["store.refresh", "store.prune"], &["store.stopped"]).is_ok());
    /// assert_eq!(
    ///     registration.check_names(&["store.refresh", "store.refresh"], &[]),
    ///     Err(RegistrationError::Duplicate("store.refresh")),
    /// );
    /// ```
    pub fn check_names(
        &self,
        components: &[&'static str],
        cleanups: &[&'static str],
    ) -> Result<(), RegistrationError> {
        self.supervisor.check_names(components, cleanups)
    }

    /// Whether this target still owns the finalizer's original cleanup stack.
    /// This does not prove finalizer execution or remote resource liveness.
    /// See [`crate::cleanup::CleanupSlot::register_tracked`] for an example.
    pub fn owns_cleanup(&self, registration: &crate::cleanup::CleanupRegistration) -> bool {
        self.supervisor.owns_cleanup(registration)
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
/// impl batter_core::registration::RegistrationTarget for Forged {
///     fn registration(&mut self) -> batter_core::registration::Registration<'_> {
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
