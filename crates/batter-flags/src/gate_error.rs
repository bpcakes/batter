//! The one refusal type of every gate and wrapper constructor.

use batter_flags_core::{FlagKey, low_level::HandleRefusal};

/// Why a gate or a job wrapper was refused when it was built.
///
/// One type serves the HTTP gate builders and the job wrapper constructors, so
/// a root handles one set of refusals whichever surface it gates. A refusal
/// happens at construction, before any traffic: evaluation, by contrast, always
/// answers, with the code default and an `error` decision, because a request
/// must get an answer.
///
/// Every variant that names a switch carries its key, a declared name and never
/// application data.
///
/// ```
/// use batter_flags::GateError;
/// use batter_flags::core::FlagKey;
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let refusal = GateError::NotOwned { key: FlagKey::new("export_halt")? };
/// assert_eq!(
///     refusal.to_string(),
///     "kill switch `export_halt` is not owned by this process kind",
/// );
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum GateError {
    /// The switch was minted by another catalog than the one these flags
    /// resolved.
    #[error("kill switch `{key}` belongs to another catalog")]
    ForeignHandle {
        /// The switch's key.
        key: FlagKey,
    },
    /// This process kind does not own the switch, so it would never be
    /// engaged here.
    #[error("kill switch `{key}` is not owned by this process kind")]
    NotOwned {
        /// The switch's key.
        key: FlagKey,
    },
    /// A switch was listed more than once in one gate.
    #[error("kill switch `{key}` is listed more than once")]
    DuplicateSwitch {
        /// The switch's key.
        key: FlagKey,
    },
}

impl GateError {
    /// The refusal for a switch the flags' base cannot read, naming `key`.
    pub(crate) fn refused(key: &FlagKey, refusal: HandleRefusal) -> Self {
        match refusal {
            HandleRefusal::ForeignCatalog => Self::ForeignHandle { key: key.clone() },
            HandleRefusal::NotOwned => Self::NotOwned { key: key.clone() },
        }
    }
}
