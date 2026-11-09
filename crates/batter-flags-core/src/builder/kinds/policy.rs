//! Consuming single-assignment policy transitions, independent of ownership.
use super::{KillSwitch, ModeDeclaration, ReleaseDeclaration};
use crate::{FlagValue, vocabulary::EnvironmentClass};
use std::marker::PhantomData;

/// A declaration policy has not been assigned. This marker has no values.
pub enum Unassigned {}

/// A declaration policy has been assigned and cannot be replaced.
pub enum Assigned {}

impl<'b, T: FlagValue, O, G> ReleaseDeclaration<'b, T, O, Unassigned, G> {
    /// Restrict this flag to one environment class; the control plane never
    /// supplies it. A setting outside that class fails startup.
    ///
    /// Migration: `only_in` now returns a declaration with an assigned
    /// environment policy. Infer its type and call `only_in` only once.
    ///
    /// ```compile_fail,E0599
    /// # use batter_flags_core::{CatalogBuilder, SettingsPrefix, Toggle, Expiry, CivilDate};
    /// # let (mut b, _serving, class) =
    /// #     CatalogBuilder::new(SettingsPrefix::new("FLAG_").unwrap(), "serving", "hosted").unwrap();
    /// b.release_flag("fast", Toggle::Off, Expiry::on(CivilDate::new(2027, 1, 1).unwrap()))
    ///     .only_in(&class).only_in(&class);
    /// ```
    pub fn only_in(
        mut self,
        class: &EnvironmentClass,
    ) -> ReleaseDeclaration<'b, T, O, Assigned, G> {
        self.draft.only_in(class);
        ReleaseDeclaration {
            draft: self.draft,
            default: self.default,
            expiry: self.expiry,
            ownership: PhantomData,
            environment: PhantomData,
            governance: PhantomData,
        }
    }
}

impl<'b, T: FlagValue, O> ModeDeclaration<'b, T, O, Unassigned> {
    /// Restrict this flag to one environment class; the control plane never
    /// supplies it. A setting outside that class fails startup.
    ///
    /// Migration: `only_in` now returns a declaration with an assigned
    /// environment policy. Infer its type and call `only_in` only once.
    ///
    /// ```compile_fail,E0599
    /// # use batter_flags_core::{CatalogBuilder, SettingsPrefix, Toggle};
    /// # let (mut b, _serving, class) =
    /// #     CatalogBuilder::new(SettingsPrefix::new("FLAG_").unwrap(), "serving", "hosted").unwrap();
    /// b.mode_flag("fast", Toggle::Off).only_in(&class).only_in(&class);
    /// ```
    pub fn only_in(mut self, class: &EnvironmentClass) -> ModeDeclaration<'b, T, O, Assigned> {
        self.draft.only_in(class);
        ModeDeclaration {
            draft: self.draft,
            default: self.default,
            ownership: PhantomData,
        }
    }
}

impl<'b, T: FlagValue, O, E> ReleaseDeclaration<'b, T, O, E, Unassigned> {
    /// Assign the governing switch once. While engaged, it overrides even a
    /// pin with the code default. Every owner and pin reader must own the switch.
    ///
    /// Migration: `governed_by` now returns a declaration with assigned
    /// governance. Infer its type and call `governed_by` only once.
    ///
    /// ```compile_fail,E0599
    /// # use batter_flags_core::{CatalogBuilder, SettingsPrefix, Toggle, Expiry, CivilDate, SafeState, StaleRule};
    /// # let (mut b, _serving, _hosted) = CatalogBuilder::new(SettingsPrefix::new("FLAG_").unwrap(), "serving", "hosted").unwrap();
    /// # let halt = b.kill_switch("halt", SafeState::Released, StaleRule::HoldLastKnown).declare().unwrap();
    /// b.release_flag("fast", Toggle::Off, Expiry::on(CivilDate::new(2027, 1, 1).unwrap()))
    ///     .governed_by(&halt).governed_by(&halt);
    /// ```
    pub fn governed_by(mut self, switch: &KillSwitch) -> ReleaseDeclaration<'b, T, O, E, Assigned> {
        self.draft.governed_by(switch);
        ReleaseDeclaration {
            draft: self.draft,
            default: self.default,
            expiry: self.expiry,
            ownership: PhantomData,
            environment: PhantomData,
            governance: PhantomData,
        }
    }
}
