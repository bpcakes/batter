use super::CleanupSlot;
use crate::BoxError;
use std::{
    future::{Future, poll_fn},
    pin::pin,
};

/// A native acquisition's handle paired with its owned finalizer.
///
/// Construct this at the successful return boundary of an acquisition factory
/// passed to [`CleanupSlot::acquire`]. There is no accessor that can release the
/// handle before its finalizer is registered. The factory still owns partial
/// acquisitions, cancellation and panic before it returns this pair; this type
/// does not prove that the finalizer closes the right remote resource.
///
/// ```compile_fail,E0616
/// use batter_core::cleanup::Acquired;
/// let acquired = Acquired::new(42, || async { Ok(()) });
/// let handle = acquired.handle;
/// ```
#[must_use = "return the acquired pair to CleanupSlot::acquire for registration"]
pub struct Acquired<T, F> {
    handle: T,
    finalizer: F,
}

impl<T, F, Fut> Acquired<T, F>
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: Future<Output = Result<(), BoxError>> + Send + 'static,
{
    /// Pair the application handle with the owned cleanup operation.
    /// No finalizer is run on drop. See [`CleanupSlot::acquire`] for execution.
    pub fn new(handle: T, finalizer: F) -> Self {
        Self { handle, finalizer }
    }
}

impl CleanupSlot<'_> {
    /// Acquire through an inert factory and register cleanup before exposing its handle.
    ///
    /// On successful factory completion, registration and handle return happen
    /// in the same poll, with no user callback or await between them. Subsequent
    /// caller failure or cancellation therefore cannot leave the returned
    /// handle unregistered. Finalizers still run LIFO under the owning command,
    /// startup or supervisor, or an explicitly driven cleanup stack.
    ///
    /// Dropping this future before completion drops the native acquisition;
    /// its partial-acquisition contract applies. A never-polled future invokes
    /// no factory. Use [`Self::register`] only for lower-level manual handoff.
    ///
    /// # Errors
    /// Returns the original acquisition error unchanged; no finalizer is added.
    /// Factory panics propagate into the enclosing execution boundary.
    ///
    /// ```
    /// use batter_core::cleanup::{Acquired, CleanupBudget, CleanupStack};
    /// use std::{convert::Infallible, sync::Arc, time::Duration};
    /// # #[tokio::main(flavor = "current_thread")]
    /// # async fn main() -> Result<(), batter_core::RegistrationError> {
    /// let mut cleanup = CleanupStack::new();
    /// let semaphore = Arc::new(tokio::sync::Semaphore::new(1));
    /// let source = semaphore.clone();
    /// let handle = cleanup.reserve("capacity")?.acquire(move || async move {
    ///     let permit = source.acquire_owned().await.expect("semaphore stays open");
    ///     Ok::<_, Infallible>(Acquired::new(42, move || async move {
    ///         drop(permit);
    ///         Ok(())
    ///     }))
    /// }).await.expect("infallible acquisition");
    /// assert_eq!(handle, 42);
    /// assert_eq!(semaphore.available_permits(), 0);
    /// let second = Duration::from_secs(1);
    /// let report = cleanup.close(CleanupBudget::new(second, second, second).unwrap()).await;
    /// assert!(report.is_success());
    /// assert_eq!(semaphore.available_permits(), 1);
    /// # Ok(()) }
    /// ```
    pub async fn acquire<T, E, A, Fut, F, Close>(self, acquire: A) -> Result<T, E>
    where
        A: FnOnce() -> Fut,
        Fut: Future<Output = Result<Acquired<T, F>, E>>,
        F: FnOnce() -> Close + Send + 'static,
        Close: Future<Output = Result<(), BoxError>> + Send + 'static,
    {
        let mut acquiring = pin!(acquire());
        let Acquired { handle, finalizer } = poll_fn(|cx| acquiring.as_mut().poll(cx)).await?;
        self.register(finalizer);
        Ok(handle)
    }
}
