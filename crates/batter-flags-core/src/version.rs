//! The control-plane version counter.

use std::fmt;

/// A control-plane version: the counter every audited write raises, in commit
/// order, starting at 0 for a store never written. It rises by one, except
/// that a write after a restore to an older head is numbered above every
/// running process that has reported its version since.
///
/// A snapshot, a decision's source and a pin's origin all name the version
/// they were read at, so two readers that report the same version read the
/// same rule set.
///
/// ```
/// use batter_flags_core::Version;
///
/// let version = Version::new(41);
/// assert!(version < Version::new(42));
/// assert_eq!(version.get(), 41);
/// assert_eq!(version.to_string(), "41");
/// ```
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(transparent)
)]
pub struct Version(u64);

impl Version {
    /// A version from its counter value.
    pub const fn new(counter: u64) -> Self {
        Self(counter)
    }

    /// The counter value.
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Display for Version {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}
