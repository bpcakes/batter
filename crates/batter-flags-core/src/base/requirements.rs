//! What a process lacks to serve a flag's variants: required settings that
//! were absent, and an environment class it runs outside. Both are recorded
//! for the kinds that may replay a flag's pins, owners and `pinned_by` kinds
//! alike, because a replayed pin is served wherever it is honoured.

use super::{BaseBuilder, BaseError};
use crate::{declaration::Kind, key::FlagKey};

impl BaseBuilder {
    /// The index of a release or mode flag this base's kind owns or lists in
    /// its `pinned_by`.
    fn replayable(&self, key: &FlagKey) -> Result<usize, BaseError> {
        let index = self.locate(key)?;
        let entry = &self.catalog.entries[index];
        if entry.kind == Kind::Switch {
            return Err(BaseError::WrongKind);
        }
        if !entry.owners.contains(&self.kind) && !entry.pinned_by.contains(&self.kind) {
            return Err(BaseError::NotOwned);
        }
        Ok(index)
    }

    /// Records that the settings an enabling `variant` requires were absent or
    /// empty. A published entry that selects the variant is then held instead
    /// of served, and a pin that selects it is refused on replay.
    ///
    /// A kind listed in the flag's `pinned_by` records it too: it never
    /// evaluates the flag live, but it serves the pins it replays.
    ///
    /// # Errors
    ///
    /// [`BaseError::UnknownKey`], [`BaseError::WrongKind`] for a kill switch,
    /// [`BaseError::NotOwned`] for a kind that neither owns the flag nor is
    /// listed in its `pinned_by`, [`BaseError::UndeclaredVariant`] and
    /// [`BaseError::NoRequiredSettings`].
    pub fn settings_absent(&mut self, key: &FlagKey, variant: &str) -> Result<(), BaseError> {
        let index = self.replayable(key)?;
        let entry = &self.catalog.entries[index];
        let declared = entry
            .variants
            .iter()
            .copied()
            .find(|declared| *declared == variant)
            .ok_or(BaseError::UndeclaredVariant)?;
        if !entry
            .requires_settings
            .iter()
            .any(|required| required.variant == declared)
        {
            return Err(BaseError::NoRequiredSettings);
        }
        let absent = &mut self.settings_absent[index];
        if !absent.contains(&declared) {
            absent.push(declared);
        }
        Ok(())
    }

    /// Records that the flag is restricted with `only_in` to an environment
    /// class this process runs outside. Such a process evaluates the flag at
    /// its code default, so a pin that selects any other variant is refused
    /// on replay. Recording it twice is the same as once.
    ///
    /// ```
    /// use batter_flags_core::{
    ///     CatalogBuilder, CivilDate, Expiry, Flag, SettingsPrefix, Toggle, low_level::BaseBuilder,
    /// };
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let (mut catalog, serving, _hosted) =
    ///     CatalogBuilder::new(SettingsPrefix::new("FLAG_")?, "serving", "hosted")?;
    /// let development = catalog.environment_class("development")?;
    /// let expiry = Expiry::on(CivilDate::new(2027, 3, 31)?);
    /// let preview = catalog
    ///     .release_flag("preview", Toggle::Off, expiry)
    ///     .only_in(&development)
    ///     .declare()?;
    ///
    /// // A hosted process serves `preview` at its code default only.
    /// let mut base = BaseBuilder::new(catalog.build(), serving)?;
    /// base.outside_class(preview.key())?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// [`BaseError::UnknownKey`], [`BaseError::WrongKind`] for a kill switch,
    /// [`BaseError::NotOwned`] for a kind that neither owns the flag nor is
    /// listed in its `pinned_by`, and [`BaseError::NotRestricted`] for a flag
    /// without `only_in`.
    pub fn outside_class(&mut self, key: &FlagKey) -> Result<(), BaseError> {
        let index = self.replayable(key)?;
        if self.catalog.entries[index].only_in.is_none() {
            return Err(BaseError::NotRestricted);
        }
        self.outside_class[index] = true;
        Ok(())
    }
}
