//! The typed flag and kill-switch catalog of `batter-flags`.
//!
//! This crate owns the flag key and catalog grammar, declaration kinds,
//! validated snapshots and their views, evaluation that reads no clock,
//! pinned decisions and the disclosed `low_level` publication path. It
//! deliberately depends on no other Batter crate and contains no async
//! runtime, transport framework or persistence integration: the `PostgreSQL`
//! store lives in `batter-flags-postgres`, and lifecycle ownership stays with
//! `batter-flags`, which registers the loader. Applications reach it as
//! `batter_flags::core` or `batter::flags::core`.
//!
//! An application declares every flag and kill switch once, in code, with a
//! [`CatalogBuilder`]. Each declaration mints a typed, catalog-branded handle
//! ([`ReleaseFlag`], [`ModeFlag`] or [`KillSwitch`]); the built [`Catalog`]
//! carries the [`CatalogDigest`] and, with the `serde` feature, the manifest
//! deployment tooling reads.
//!
//! ```
//! use batter_flags_core::{
//!     CatalogBuilder, CivilDate, Expiry, SafeState, SettingsPrefix, StaleRule, Toggle,
//! };
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let mut builder = CatalogBuilder::new(SettingsPrefix::new("FLAG_")?);
//! let serving = builder.process_kind("serving")?;
//! builder.environment_class("hosted")?;
//! let halt = builder
//!     .kill_switch("checkout_halt", SafeState::Released, StaleRule::HoldLastKnown)
//!     .declare()?;
//! let _new_checkout = builder
//!     .release_flag("new_checkout", Toggle::Off, Expiry::on(CivilDate::new(2027, 3, 31)?))
//!     .owned_only_by(&serving)
//!     .governed_by(&halt)
//!     .declare()?;
//! let catalog = builder.build()?;
//! assert_eq!(catalog.declarations().count(), 2);
//! # Ok(())
//! # }
//! ```
//!
//! The crate is implemented through evaluation: keys and setting names, value
//! types, the three declaration kinds with their per-declaration policy,
//! vocabularies, branded handles, the catalog, declaration, revision and
//! snapshot digests, expiry, validated [`Snapshot`](low_level::Snapshot)s whose
//! rows are published or held one by one, [`SnapshotView`] evaluation that
//! returns a layered [`Decision`], [`Pinned`] decisions with the
//! [`PinRefusal`] a rejected one must be answered through, the three-way
//! [`GateReading`](low_level::GateReading) a gate acts on, the
//! [`Authority`](low_level::Authority) transitions and the one immutable
//! [`Published`](low_level::Published) record a loader follows, the
//! `feature_flag.evaluation` event and, with the `serde` feature, the manifest,
//! its digest and `manifest_diff`. Relationships between flags are stated on the
//! builder, with [`CatalogBuilder::requires`].
//!
//! Views are built through the disclosed [`low_level`] path, which asserts a
//! source and a freshness and is weaker than the protected one. A built surface
//! can refuse a handle once, at construction, instead of answering an error at
//! every evaluation: [`Base::check_flag`](low_level::Base::check_flag) and
//! [`Base::check_switch`](low_level::Base::check_switch) apply the rule
//! evaluation applies. Declared-mode resolution, the reader that publishes
//! snapshots and the test flags are in `batter-flags`; every loader arrives
//! with a later delivery task of the flags epic.
//!
//! The `serde` feature derives serialization for the manifest and snapshot
//! types that the store and the operator command exchange; without it the
//! normal dependency closure holds no serialization library.
//!
//! Every public enum is exhaustive, errors included, so a new variant is a
//! recorded breaking change.
#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod authority;
mod base;
mod builder;
mod candidate;
mod catalog;
mod check;
mod date;
mod decision;
mod declaration;
mod digest;
mod encoding;
mod error;
mod events;
mod handle;
mod invariants;
mod key;
pub mod low_level;
#[cfg(feature = "serde")]
mod manifest;
mod pin;
mod published;
#[cfg(feature = "serde")]
mod serde_support;
mod snapshot;
mod value;
mod version;
mod view;
mod vocabulary;

pub use builder::{
    CatalogBuilder, Everyone, ModeDeclaration, ReleaseDeclaration, Restricted, SwitchDeclaration,
};
pub use catalog::{Catalog, Retired};
pub use date::{CivilDate, CivilDateError, Expiry};
pub use decision::{
    Decision, ErrorCause, Freshness, KillSwitchKey, Layer, PrePublication, Reason, Source,
    SwitchState,
};
pub use declaration::{
    Absence, Declared, FlipDiscipline, Invariant, InvariantRule, Kind, Owners, SafeState, StaleRule,
};
#[cfg(feature = "serde")]
pub use digest::ManifestDigest;
pub use digest::{
    CatalogDigest, DeclarationFingerprint, DigestParseError, Revision, SnapshotDigest,
};
pub use error::{BuildError, ForeignKind};
pub use handle::{Flag, KillSwitch, ModeFlag, ReleaseFlag};
pub use invariants::Standing;
pub use key::{
    FlagKey, KeyError, MAX_KEY_LENGTH, MAX_PREFIX_LENGTH, MAX_SETTING_NAME_LENGTH, PrefixError,
    SettingName, SettingNameError, SettingsPrefix,
};
#[cfg(feature = "serde")]
pub use manifest::{
    InvariantHold, Manifest, ManifestDiff, ManifestDiffError, ManifestEntry, ManifestInvariant,
    ManifestOwners, ManifestRequiredSettings, ManifestRetired, Retype, manifest_diff,
};
pub use pin::{PinOrigin, PinRefusal, PinRejection, Pinnable, Pinned, PinnedResolution};
pub use value::{FlagValue, Toggle, ValueType, VariantsError};
pub use version::Version;
pub use view::SnapshotView;
pub use vocabulary::{EnvironmentClass, ProcessKind};
