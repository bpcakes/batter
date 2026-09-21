use thiserror::Error;

use crate::{Capacity, CounterKey, FixedWindowPolicy, PolicySubject, RateLimitPolicy, SubjectKey};

/// One proposed quota charge against a policy and opaque subject.
///
/// A newly constructed check has cost 1. Custom costs are validated against
/// the referenced policy so a backend never receives a zero-cost or
/// intrinsically impossible check.
#[derive(Debug, Eq, PartialEq)]
pub struct Check<'a, P: RateLimitPolicy + ?Sized = FixedWindowPolicy> {
    policy: &'a P,
    subject: SubjectKey,
    cost: u64,
}

impl<P: RateLimitPolicy + ?Sized> Copy for Check<'_, P> {}

impl<P: RateLimitPolicy + ?Sized> Clone for Check<'_, P> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<'a, P: RateLimitPolicy + ?Sized> Check<'a, P> {
    /// Constructs a check with the default cost of 1.
    ///
    /// The [`PolicySubject`] already carries the exact policy used for subject
    /// derivation, so this constructor has no independent policy argument that
    /// could accidentally disagree with it. A caller can leave this safe path
    /// only through an explicitly unbound [`SubjectKey`].
    pub const fn new(subject: PolicySubject<'a, P>) -> Self {
        let (policy, subject) = subject.into_parts();
        Self {
            policy,
            subject,
            cost: 1,
        }
    }

    /// Returns this check with a validated custom cost.
    ///
    /// # Errors
    ///
    /// Returns an error when `cost` is zero or exceeds the policy capacity.
    pub fn with_cost(mut self, cost: u64) -> Result<Self, CheckError> {
        validate_cost(self.policy, cost)?;
        self.cost = cost;
        Ok(self)
    }

    /// Returns the policy evaluated by this check.
    pub const fn policy(&self) -> &'a P {
        self.policy
    }

    /// Returns an unbound copy of the opaque subject key evaluated by this
    /// check.
    ///
    /// This accessor exists for backend and adapter integration. Binding the
    /// returned key to another policy is an explicit escape from the
    /// derivation-to-check guarantee described by [`Check::new`].
    pub const fn subject(&self) -> SubjectKey {
        self.subject
    }

    /// Returns the complete logical identity of the stored counter.
    pub fn counter_key(&self) -> CounterKey {
        CounterKey::new(self.policy.fingerprint(), self.subject)
    }

    /// Returns the nonzero quota cost.
    pub const fn cost(&self) -> u64 {
        self.cost
    }
}

fn validate_cost<P: RateLimitPolicy + ?Sized>(policy: &P, cost: u64) -> Result<(), CheckError> {
    if cost == 0 {
        return Err(CheckError::ZeroCost);
    }
    let capacity = policy.capacity();
    if cost > capacity.get() {
        return Err(CheckError::CostExceedsCapacity { cost, capacity });
    }
    Ok(())
}

/// An invalid check cost.
#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum CheckError {
    /// The requested cost was zero.
    #[error("check cost must be greater than zero")]
    ZeroCost,
    /// The requested cost exceeded the referenced policy's capacity.
    #[error("check cost ({cost}) exceeds the policy capacity ({capacity})")]
    CostExceedsCapacity {
        /// Requested cost.
        cost: u64,
        /// Maximum cost accepted by the policy.
        capacity: Capacity,
    },
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{Check, CheckError};
    use crate::{Capacity, FixedWindowPolicy, PolicyId, ScopeId, SubjectKey};

    fn policy() -> FixedWindowPolicy {
        FixedWindowPolicy::new(
            PolicyId::new("auth.login").unwrap(),
            ScopeId::new("client").unwrap(),
            8,
            Duration::from_mins(1),
        )
        .unwrap()
    }

    #[test]
    fn defaults_to_one_unit_of_cost() {
        let policy = policy();
        let subject = SubjectKey::from_digest([1; 32]);
        let check = Check::new(subject.bind(&policy));

        assert_eq!(check.policy(), &policy);
        assert_eq!(check.subject(), subject);
        assert_eq!(check.cost(), 1);
    }

    #[test]
    fn accepts_cost_up_to_and_including_the_limit() {
        let policy = policy();
        let subject = SubjectKey::from_digest([2; 32]);

        assert_eq!(
            Check::new(subject.bind(&policy))
                .with_cost(3)
                .unwrap()
                .cost(),
            3
        );
        assert_eq!(
            Check::new(subject.bind(&policy))
                .with_cost(policy.limit().get())
                .unwrap()
                .cost(),
            policy.limit().get()
        );
    }

    #[test]
    fn rejects_zero_cost() {
        let policy = policy();

        assert_eq!(
            Check::new(SubjectKey::from_digest([3; 32]).bind(&policy)).with_cost(0),
            Err(CheckError::ZeroCost)
        );
    }

    #[test]
    fn rejects_cost_above_policy_limit() {
        let policy = policy();

        assert_eq!(
            Check::new(SubjectKey::from_digest([4; 32]).bind(&policy)).with_cost(9),
            Err(CheckError::CostExceedsCapacity {
                cost: 9,
                capacity: Capacity::new(8).unwrap(),
            })
        );
    }
}
