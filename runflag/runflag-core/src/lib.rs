//! Framework-neutral typed flags and kill switches.
//!
//! This package owns the flag key and catalog grammar, declaration kinds,
//! validated snapshots and their views, evaluation that reads no clock,
//! pinned decisions and the disclosed `low_level` publication path. It
//! deliberately contains no async runtime, transport framework or persistence
//! integration: the `PostgreSQL` store lives in `runflag-postgres`, and
//! lifecycle ownership stays with the adapter that registers a loader.
//!
//! An application declares every flag and kill switch once, in code, with a
//! [`CatalogBuilder`]. Each declaration mints a typed, catalog-branded handle
//! ([`ReleaseFlag`], [`ModeFlag`] or [`KillSwitch`]); the built [`Catalog`]
//! carries the [`CatalogDigest`] and, with the `serde` feature, the manifest
//! deployment tooling reads.
//!
//! ```
//! use runflag_core::{
//!     CatalogBuilder, CivilDate, Expiry, Owners, SafeState, SettingsPrefix, StaleRule, Toggle,
//!     ToggleLiterals,
//! };
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let mut builder = CatalogBuilder::new(SettingsPrefix::new("FLAG_")?, ToggleLiterals::TrueFalse);
//! let serving = builder.process_kind("serving")?;
//! builder.environment_class("hosted")?;
//! let halt = builder
//!     .kill_switch("checkout_halt", SafeState::Released, StaleRule::HoldLastKnown)
//!     .declare()?;
//! let _new_checkout = builder
//!     .release_flag("new_checkout", Toggle::Off, Expiry::on(CivilDate::new(2027, 3, 31)?))
//!     .owners(Owners::Only(&[serving]))
//!     .governed_by(&halt)
//!     .declare()?;
//! let catalog = builder.build()?;
//! assert_eq!(catalog.declarations().count(), 2);
//! # Ok(())
//! # }
//! ```
//!
//! The catalog is implemented: keys and setting names, value types, the
//! three declaration kinds with their per-declaration policy, vocabularies,
//! branded handles, the catalog and declaration digests, expiry and, with the
//! `serde` feature, the manifest, its digest and `manifest_diff`. Snapshots,
//! views, evaluation, pinned decisions and the `low_level` publication path
//! are not implemented yet; they arrive with a later delivery task of the
//! flags epic.
//!
//! The `serde` feature derives serialization for the manifest and snapshot
//! types that the store and the operator command exchange; without it the
//! normal dependency closure holds no serialization library.
//!
//! Every public enum is exhaustive, errors included, so a new variant is a
//! recorded breaking change.

mod builder;
mod catalog;
mod date;
mod declaration;
mod digest;
mod error;
mod handle;
mod key;
#[cfg(feature = "serde")]
mod manifest;
#[cfg(feature = "serde")]
mod serde_support;
mod value;
mod vocabulary;

pub use builder::{CatalogBuilder, ModeDeclaration, ReleaseDeclaration, SwitchDeclaration};
pub use catalog::{Catalog, Retired};
pub use date::{CivilDate, CivilDateError, Expiry};
pub use declaration::{
    Absence, Declared, FlipDiscipline, Invariant, InvariantRule, Kind, Owners, SafeState, StaleRule,
};
#[cfg(feature = "serde")]
pub use digest::ManifestDigest;
pub use digest::{CatalogDigest, DeclarationFingerprint, DigestParseError};
pub use error::{BuildError, ForeignKind};
pub use handle::{Flag, KillSwitch, ModeFlag, ReleaseFlag};
pub use key::{
    FlagKey, KeyError, MAX_KEY_LENGTH, MAX_PREFIX_LENGTH, MAX_SETTING_NAME_LENGTH, PrefixError,
    SettingName, SettingNameError, SettingsPrefix,
};
#[cfg(feature = "serde")]
pub use manifest::{
    Manifest, ManifestDiff, ManifestDiffError, ManifestEntry, ManifestInvariant, ManifestOwners,
    ManifestRequiredSettings, ManifestRetired, Retype, manifest_diff,
};
pub use value::{FlagValue, Toggle, ToggleLiterals, ValueType, VariantsError};
pub use vocabulary::{EnvironmentClass, ProcessKind};
