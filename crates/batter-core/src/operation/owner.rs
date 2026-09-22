use super::OperationContext;
use crate::{ConfigurationError, validation};
use std::time::Duration;
use tokio::time::Instant;

/// An explicitly independent root's absolute deadline.
///
/// This value does not witness a parent relationship. Derive children from an
/// existing [`OperationContext`] instead. Reusing this value never resets time.
///
/// ```
/// use batter_core::operation::{OperationOwner, RootDeadline};
/// use std::time::Duration;
/// let deadline = RootDeadline::after(Duration::from_secs(1))?;
/// let owner = OperationOwner::at(deadline);
/// assert_eq!(owner.context().deadline(), deadline.instant());
/// # Ok::<(), batter_core::ConfigurationError>(())
/// ```
#[derive(Clone, Copy, Debug)]
pub struct RootDeadline(Instant);

impl RootDeadline {
    /// Start an independent root budget now, rejecting zero and overflow.
    pub fn after(budget: Duration) -> Result<Self, ConfigurationError> {
        validation::positive(budget, "operation budget")?;
        Instant::now()
            .checked_add(budget)
            .map(Self)
            .ok_or(ConfigurationError::TooLarge("operation budget"))
    }

    /// Explicitly select an independent root deadline. Past deadlines are expired.
    /// This cannot establish inheritance from another operation.
    pub fn at(deadline: Instant) -> Self {
        Self(deadline)
    }

    /// Read the absolute runtime instant without creating child authority.
    pub fn instant(self) -> Instant {
        self.0
    }
}

/// Explicit cancellation authority for one root or derived child operation.
///
/// Share [`Self::context`] with work; context clones cannot cancel this scope.
/// The owner is not cloneable. Dropping it alone does not request cancellation
/// or join work. Execution boundaries still cancel their own children on exit.
///
/// ```
/// use batter_core::operation::{Interruption, OperationOwner};
/// use std::time::Duration;
/// let root = OperationOwner::new(Duration::from_secs(1))?;
/// let context = root.context().clone();
/// let child = context.child(Duration::from_secs(2))?;
/// child.cancel();
/// assert_eq!(child.context().check(), Err(Interruption::Cancelled));
/// assert!(context.check().is_ok());
/// root.cancel();
/// assert_eq!(context.check(), Err(Interruption::Cancelled));
/// # Ok::<(), batter_core::ConfigurationError>(())
/// ```
///
/// ```compile_fail,E0599
/// use batter_core::operation::OperationOwner;
/// fn cannot_duplicate_authority(owner: OperationOwner) { let _ = owner.clone(); }
/// ```
#[derive(Debug)]
#[must_use = "retain cancellation authority or explicitly project its context"]
pub struct OperationOwner {
    pub(super) context: OperationContext,
}

impl OperationOwner {
    /// Create an independent root with a positive total budget starting now.
    pub fn new(budget: Duration) -> Result<Self, ConfigurationError> {
        RootDeadline::after(budget).map(Self::at)
    }

    /// Create an independent root with the already selected absolute deadline.
    pub fn at(deadline: RootDeadline) -> Self {
        Self {
            context: OperationContext::at(deadline.instant()),
        }
    }

    /// Borrow execution and observation capability without cancellation authority.
    pub fn context(&self) -> &OperationContext {
        &self.context
    }

    /// Give up explicit cancellation authority and retain only the context.
    pub fn into_context(self) -> OperationContext {
        self.context
    }

    /// Cancel this scope and its descendants, never its parent or siblings.
    pub fn cancel(&self) {
        self.context.cancel();
    }
}
