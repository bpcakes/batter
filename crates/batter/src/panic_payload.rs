use std::{
    any::Any,
    fmt,
    sync::{Mutex, TryLockError},
};

/// Retained unwinding panic payload; default formatting never inspects it.
///
/// This root path is shared by startup, command, and managed lifecycle reports.
/// The original [`crate::startup::PanicPayload`] path remains available.
pub struct PanicPayload(Mutex<Box<dyn Any + Send>>);

/// Another caller is currently inspecting the retained panic payload.
///
/// This is a transient local contention result, not evidence about the payload
/// or the operation that panicked. The original
/// [`crate::startup::PanicPayloadBusy`] path remains available.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("panic payload is already being inspected")]
pub struct PanicPayloadBusy;

impl PanicPayload {
    pub(crate) fn new(payload: Box<dyn Any + Send>) -> Self {
        Self(Mutex::new(payload))
    }

    /// Inspect the original payload in a trusted callback without waiting for a lock.
    ///
    /// Concurrent or recursive inspection returns [`PanicPayloadBusy`] instead
    /// of blocking. A previous callback panic does not prevent later inspection.
    /// Keep the callback short; it temporarily excludes other inspectors.
    ///
    /// ```
    /// use batter::{PanicPayload, PanicPayloadBusy};
    ///
    /// fn is_string(payload: &PanicPayload) -> Result<bool, PanicPayloadBusy> {
    ///     payload.try_inspect(|value| value.is::<&'static str>())
    /// }
    /// ```
    pub fn try_inspect<T>(
        &self,
        inspect: impl FnOnce(&(dyn Any + Send)) -> T,
    ) -> Result<T, PanicPayloadBusy> {
        let payload = match self.0.try_lock() {
            Ok(payload) => payload,
            Err(TryLockError::Poisoned(error)) => error.into_inner(),
            Err(TryLockError::WouldBlock) => return Err(PanicPayloadBusy),
        };
        Ok(inspect(&**payload))
    }
}

impl fmt::Debug for PanicPayload {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Preserve the established redacted diagnostic for compatibility.
        f.write_str("startup panic payload retained")
    }
}
