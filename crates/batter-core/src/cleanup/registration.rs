//! Registration evidence for adapter-owned dependencies.
use super::{CleanupFuture, CleanupSlot, CleanupStack, Hook};
use crate::BoxError;
use std::{future::Future, sync::Arc};

/// Evidence that a finalizer was installed in one particular cleanup stack.
///
/// Created by [`CleanupSlot::register_tracked`]. This proves registration,
/// not execution or the meaning of a caller-supplied finalizer. Adapters keep
/// it with their resource to check ownership through
/// [`crate::registration::Registration::owns_cleanup`]. Moving the stack
/// preserves its identity; extracting it from a supervisor transfers ownership.
///
/// ```compile_fail,E0451
/// let forged = batter_core::cleanup::CleanupRegistration {
///     owner: std::sync::Arc::new(()),
/// };
/// ```
#[derive(Clone, Debug)]
pub struct CleanupRegistration {
    owner: Arc<()>,
}

impl CleanupSlot<'_> {
    /// Install a finalizer and return evidence of its owning stack.
    ///
    /// Like [`Self::register`], this is manual acquisition glue. The evidence
    /// cannot certify that an arbitrary callback closes the right resource.
    ///
    /// ```
    /// # use batter_core::{cleanup::CleanupBudget, lifecycle::{ShutdownBudget, Supervisor}, registration::RegistrationTarget};
    /// # let second = std::time::Duration::from_secs(1);
    /// # let cleanup = CleanupBudget::new(second, second, second).unwrap();
    /// # let mut owner = Supervisor::new(ShutdownBudget::new(second, second, second, cleanup).unwrap());
    /// let proof = owner.reserve_cleanup("resource").unwrap()
    ///     .register_tracked(|| async { Ok(()) });
    /// assert!(owner.registration().owns_cleanup(&proof));
    /// let extracted = owner.take_cleanup();
    /// assert!(!owner.registration().owns_cleanup(&proof));
    /// # drop(extracted);
    /// ```
    pub fn register_tracked<F, Fut>(self, action: F) -> CleanupRegistration
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = Result<(), BoxError>> + Send + 'static,
    {
        self.stack.hooks.push(Hook {
            name: self.name,
            action: Box::new(move || Box::pin(action()) as CleanupFuture),
        });
        CleanupRegistration {
            owner: self.stack.owner.clone(),
        }
    }
}

impl CleanupStack {
    pub(crate) fn owns_registration(&self, registration: &CleanupRegistration) -> bool {
        Arc::ptr_eq(&self.owner, &registration.owner)
    }
}
