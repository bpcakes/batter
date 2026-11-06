use super::CleanupSlot;
use std::{
    future::{Future, poll_fn},
    io,
    net::SocketAddr,
    pin::pin,
    sync::Arc,
};
use tokio::{
    net::UdpSocket,
    sync::{AcquireError, Semaphore},
};

impl CleanupSlot<'_> {
    /// Acquire one native semaphore permit and retain it until owned cleanup.
    ///
    /// The permit cannot escape or be forgotten by the caller. No application
    /// callback runs between native acquisition and registration. Cancellation
    /// while waiting has Tokio's queue-position semantics and registers nothing.
    /// Migration: instead of `CleanupSlot::acquire` or manual permit registration,
    /// pass the semaphore directly to this method.
    ///
    /// # Errors
    /// Returns Tokio's acquisition error if the semaphore is closed.
    ///
    /// ```
    /// use batter_core::cleanup::{CleanupStack, CleanupBudget};
    /// use std::{sync::Arc, time::Duration};
    /// # #[tokio::main(flavor = "current_thread")]
    /// # async fn main() -> Result<(), batter_core::BoxError> {
    /// let capacity = Arc::new(tokio::sync::Semaphore::new(1));
    /// let mut cleanup = CleanupStack::new();
    /// cleanup.reserve("capacity")?.acquire_semaphore(capacity.clone()).await?;
    /// assert_eq!(capacity.available_permits(), 0);
    /// let second = Duration::from_secs(1);
    /// assert!(cleanup.close(CleanupBudget::new(second, second, second)?).await.is_success());
    /// assert_eq!(capacity.available_permits(), 1);
    /// # Ok(()) }
    /// ```
    /// Arbitrary acquisition glue is not accepted:
    /// ```compile_fail,E0308
    /// use batter_core::cleanup::CleanupSlot;
    /// async fn invalid(slot: CleanupSlot<'_>) {
    ///     slot.acquire_semaphore(|| async { tokio::task::yield_now().await }).await;
    /// }
    /// ```
    /// The permit cannot escape or be forgotten:
    /// ```compile_fail,E0599
    /// use batter_core::cleanup::CleanupSlot;
    /// async fn invalid(slot: CleanupSlot<'_>, capacity: std::sync::Arc<tokio::sync::Semaphore>) {
    ///     let permit = slot.acquire_semaphore(capacity).await.unwrap();
    ///     permit.forget();
    /// }
    /// ```
    pub async fn acquire_semaphore(self, semaphore: Arc<Semaphore>) -> Result<(), AcquireError> {
        drop(self.acquire_native(semaphore.acquire_owned()).await?);
        Ok(())
    }

    /// Bind one numeric UDP address and register the library's socket reference
    /// before returning a shared native handle. No DNS or application callbacks.
    ///
    /// Cleanup drops its reference; all caller clones must also be dropped to
    /// close the socket. No detached-task join or remote-effect guarantee follows.
    /// A cancelled, never-polled call does not bind. The Tokio I/O driver must be enabled.
    /// Migration: replace UDP-binding `CleanupSlot::acquire` callbacks with this
    /// operation; generic pairing belongs to [`super::low_level::acquire_pair`].
    ///
    /// # Errors
    /// Returns the native bind error unchanged.
    ///
    /// ```
    /// use batter_core::cleanup::{CleanupStack, CleanupBudget};
    /// use std::time::Duration;
    /// # #[tokio::main(flavor = "current_thread")]
    /// # async fn main() -> Result<(), batter_core::BoxError> {
    /// let mut cleanup = CleanupStack::new();
    /// let socket = cleanup.reserve("socket")?.bind_udp("127.0.0.1:0".parse()?).await?;
    /// assert!(socket.local_addr()?.port() != 0);
    /// drop(socket);
    /// let second = Duration::from_secs(1);
    /// assert!(cleanup.close(CleanupBudget::new(second, second, second)?).await.is_success());
    /// # Ok(()) }
    /// ```
    pub async fn bind_udp(self, address: SocketAddr) -> io::Result<Arc<UdpSocket>> {
        self.acquire_native(UdpSocket::bind(address)).await
    }

    // Only supported native methods may enter this kernel. It polls the actual
    // native future, retains its output, registers owned cleanup, and only then
    // destroys the ready future. No user projection or finalizer builder runs.
    async fn acquire_native<R, E>(
        self,
        native: impl Future<Output = Result<R, E>>,
    ) -> Result<Arc<R>, E>
    where
        R: Send + Sync + 'static,
    {
        let resource;
        {
            let mut native = pin!(native);
            resource = Arc::new(poll_fn(|cx| native.as_mut().poll(cx)).await?);
            let closing = resource.clone();
            self.register(move || async move {
                drop(closing);
                Ok(())
            });
            // Destroy the native future before constructing the return value.
            // On unwind, the outer local releases its reference; cleanup retains its own.
        }
        Ok(resource)
    }
}

#[cfg(test)]
#[path = "acquisition_tests.rs"]
mod tests;
