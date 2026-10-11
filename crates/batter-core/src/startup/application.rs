use crate::BoxError;
use std::{error::Error, fmt};

/// Dynamic application failure for protected initializers that combine
/// unrelated error types with `?`.
///
/// `?` converts every error that converts into [`BoxError`]: any concrete
/// `Error + Send + Sync + 'static`, `BoxError` itself, `&str` and `String`.
/// The box is retained unchanged, so a concrete cause remains reachable through
/// [`Self::downcast_ref`], [`Self::get_ref`] and [`Self::into_inner`], and as the
/// `source()` of [`super::InitializationError::Application`]. A third-party
/// dynamic error converts through its own `Into<BoxError>` implementation,
/// which may wrap its head error so that it is no longer directly
/// downcastable; apply `?` to the concrete error to keep it downcastable.
///
/// Formatting is fixed text and never inspects the cause. The type does not
/// implement `std::error::Error`: that keeps the blanket `From` conversion
/// coherent. [`super::InitializationError`]`<ApplicationError>` implements it
/// instead, so the protected startup failure still propagates with `?`.
/// Prefer a concrete error type when callers match on it; this type is for
/// initializers that would otherwise need an adapter.
///
/// ```
/// use batter_core::startup::{ApplicationError, InitializationError};
///
/// fn parse(value: &str) -> Result<u16, ApplicationError> {
///     Ok(value.parse::<u16>()?)
/// }
///
/// let error = parse("not-a-port").unwrap_err();
/// assert!(error.downcast_ref::<std::num::ParseIntError>().is_some());
/// assert_eq!(error.to_string(), "application initialization failed");
/// let failure = InitializationError::Application(error);
/// let source = std::error::Error::source(&failure).unwrap();
/// assert!(source.is::<std::num::ParseIntError>());
/// ```
pub struct ApplicationError(BoxError);

impl<E: Into<BoxError>> From<E> for ApplicationError {
    fn from(error: E) -> Self {
        Self(error.into())
    }
}

impl ApplicationError {
    /// Borrow the retained cause for trusted inspection.
    pub fn get_ref(&self) -> &(dyn Error + Send + Sync + 'static) {
        &*self.0
    }

    /// Borrow the retained cause as `C` if that is its concrete type.
    pub fn downcast_ref<C: Error + 'static>(&self) -> Option<&C> {
        self.0.downcast_ref()
    }

    /// Recover the retained cause.
    pub fn into_inner(self) -> BoxError {
        self.0
    }
}

impl fmt::Display for ApplicationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("application initialization failed")
    }
}

impl fmt::Debug for ApplicationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
