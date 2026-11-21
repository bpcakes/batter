//! Construction-time checks: whether a base can answer for a handle at all.
//!
//! Evaluation answers a handle it cannot serve with the code default and an
//! `error` decision, because a request must always get an answer. A surface
//! that is built once, such as a gate over a kill switch or a fixture that
//! injects a value, can instead refuse the handle before any traffic arrives.
//! These checks apply the rule evaluation applies, through the same function,
//! and neither evaluate nor emit an event.

use crate::{
    Catalog,
    base::{Base, BaseBuilder, BaseInner},
    decision::ErrorCause,
    handle::{Flag, Handle, KillSwitch},
    vocabulary::ProcessKind,
};

/// Who may evaluate or replay a handle.
#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) enum Reach {
    /// Owners only: live evaluation.
    Owner,
    /// Owners and the kinds a flag lists in `pinned_by`: pin replay.
    OwnerOrPinner,
}

/// Why a base cannot answer for a handle.
///
/// ```
/// use batter_flags_core::{
///     CatalogBuilder, SafeState, SettingsPrefix, StaleRule,
///     low_level::{BaseBuilder, HandleRefusal},
/// };
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let (mut catalog, serving, _) = CatalogBuilder::new(SettingsPrefix::new("FLAG_")?, "serving", "hosted")?;
/// let worker = catalog.process_kind("worker")?;
/// let halt = catalog
///     .kill_switch("halt", SafeState::Released, StaleRule::HoldLastKnown)
///     .owned_only_by(&serving)
///     .declare()?;
/// let base = BaseBuilder::new(catalog.build(), worker)?.finish();
///
/// // The worker does not own the switch, and a gate over it is refused up front.
/// assert_eq!(base.check_switch(&halt), Err(HandleRefusal::NotOwned));
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum HandleRefusal {
    /// The handle belongs to another catalog.
    #[error("the handle belongs to another catalog")]
    ForeignCatalog,
    /// This base's process kind does not own the declaration.
    #[error("this process kind does not own the declaration")]
    NotOwned,
}

impl HandleRefusal {
    /// The cause an evaluation that met this refusal reports.
    pub(crate) fn cause(self) -> ErrorCause {
        match self {
            Self::ForeignCatalog => ErrorCause::ForeignCatalog,
            Self::NotOwned => ErrorCause::NotOwned,
        }
    }
}

/// Why `kind` cannot answer for `handle` over `catalog`, if it cannot.
///
/// The one rule: a view's evaluation, a pin replay, a finished base's checks
/// and a builder's checks all apply it, so they cannot disagree. Its result is
/// the closed [`HandleRefusal`], not an [`ErrorCause`], so a caller that maps it
/// matches both variants and a new refusal cannot be mistaken for an old one.
pub(crate) fn refusal(
    (catalog, kind): (&Catalog, &ProcessKind),
    handle: &Handle,
    reach: Reach,
) -> Option<HandleRefusal> {
    if handle.catalog() != catalog.id {
        return Some(HandleRefusal::ForeignCatalog);
    }
    let entry = &catalog.entries[handle.index()];
    let reaches = entry.owners.contains(kind)
        || (reach == Reach::OwnerOrPinner && entry.pinned_by.contains(kind));
    if reaches {
        None
    } else {
        Some(HandleRefusal::NotOwned)
    }
}

fn owned(scope: (&Catalog, &ProcessKind), handle: &Handle) -> Result<(), HandleRefusal> {
    refusal(scope, handle, Reach::Owner).map_or(Ok(()), Err)
}

impl BaseInner {
    /// Why this base cannot answer for `handle`, if it cannot.
    pub(crate) fn refusal(&self, handle: &Handle, reach: Reach) -> Option<HandleRefusal> {
        refusal((&self.catalog, &self.kind), handle, reach)
    }
}

impl Base {
    /// Whether this base can evaluate `flag` live: the handle is this
    /// catalog's and this base's process kind owns the declaration.
    ///
    /// This is the rule [`SnapshotView::evaluate`](crate::SnapshotView::evaluate)
    /// applies, checked once instead of at every evaluation, and it emits no
    /// event.
    ///
    /// # Errors
    ///
    /// [`HandleRefusal::ForeignCatalog`] for another catalog's handle and
    /// [`HandleRefusal::NotOwned`] for a declaration this kind does not own.
    pub fn check_flag<F: Flag>(&self, flag: &F) -> Result<(), HandleRefusal> {
        owned((&self.inner.catalog, &self.inner.kind), flag.handle())
    }

    /// Whether this base can read `switch`, by the rule of
    /// [`check_flag`](Self::check_flag).
    ///
    /// # Errors
    ///
    /// [`HandleRefusal::ForeignCatalog`] and [`HandleRefusal::NotOwned`].
    pub fn check_switch(&self, switch: &KillSwitch) -> Result<(), HandleRefusal> {
        owned((&self.inner.catalog, &self.inner.kind), switch.handle())
    }
}

impl BaseBuilder {
    /// [`Base::check_flag`], before the base is finished: a recorder that
    /// takes handles can refuse another catalog's, which a key lookup alone
    /// would match by name.
    ///
    /// # Errors
    ///
    /// [`HandleRefusal::ForeignCatalog`] and [`HandleRefusal::NotOwned`].
    pub fn check_flag<F: Flag>(&self, flag: &F) -> Result<(), HandleRefusal> {
        owned((self.catalog(), self.process_kind()), flag.handle())
    }

    /// [`Base::check_switch`], before the base is finished.
    ///
    /// # Errors
    ///
    /// [`HandleRefusal::ForeignCatalog`] and [`HandleRefusal::NotOwned`].
    pub fn check_switch(&self, switch: &KillSwitch) -> Result<(), HandleRefusal> {
        owned((self.catalog(), self.process_kind()), switch.handle())
    }
}
