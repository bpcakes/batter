//! The declared base: what one process resolved from its settings.
//!
//! A [`Base`] owns the catalog it was built from, the process kind it was
//! resolved for and, per declaration, the literal the process's settings
//! supplied, if any. It is the bottom layer of every view: a flag with no
//! published entry serves it, and a kill switch's settings floor lives in it.
//! Its two [`Revision`]s are the stable, order-insensitive fingerprint of
//! what the process resolved.

use std::{fmt, sync::Arc};

use thiserror::Error;

use crate::{
    Catalog,
    decision::{ErrorCause, SwitchState},
    declaration::{Declared, Kind, OwnedOwners},
    digest::Revision,
    events::ErrorTally,
    key::FlagKey,
    snapshot::ValidationScope,
    vocabulary::ProcessKind,
};

/// What a process's settings supplied for one declaration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BaseState {
    /// A release or mode flag's literal, as an index into its variants.
    Flag(Option<usize>),
    /// A kill switch's literal.
    Switch(Option<SwitchState>),
}

/// The shared, immutable body of a [`Base`].
pub(crate) struct BaseInner {
    pub(crate) catalog: Catalog,
    pub(crate) kind: ProcessKind,
    /// Per declaration, in declaration order.
    pub(crate) states: Box<[BaseState]>,
    /// Per declaration, the variants whose required settings were absent or
    /// empty when the process resolved.
    pub(crate) settings_absent: Box<[Vec<&'static str>]>,
    pub(crate) revision: Revision,
    pub(crate) shared_revision: Revision,
    pub(crate) tally: ErrorTally,
}

/// The declared base of one process: its catalog, its kind and the settings
/// it resolved.
///
/// Built once with [`BaseBuilder`], which consumes the catalog. A base reads
/// no settings itself; whoever resolves settings reports each literal to the
/// builder.
///
/// ```
/// use batter_flags_core::{
///     CatalogBuilder, CivilDate, Expiry, Flag, SafeState, SettingsPrefix, StaleRule, Toggle,
///     low_level::{BaseBuilder, ResolutionSource},
/// };
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let mut catalog = CatalogBuilder::new(SettingsPrefix::new("FLAG_")?);
/// let serving = catalog.process_kind("serving")?;
/// catalog.environment_class("hosted")?;
/// let expiry = Expiry::on(CivilDate::new(2027, 3, 31)?);
/// let new_checkout = catalog.release_flag("new_checkout", Toggle::Off, expiry).declare()?;
/// catalog.kill_switch("halt", SafeState::Released, StaleRule::HoldLastKnown).declare()?;
///
/// let mut base = BaseBuilder::new(catalog.build()?, serving)?;
/// base.flag(new_checkout.key(), "on")?;
/// let base = base.finish();
///
/// let resolved: Vec<_> = base
///     .resolved()
///     .map(|resolution| (resolution.key().to_string(), resolution.variant(), resolution.source()))
///     .collect();
/// assert_eq!(
///     resolved,
///     [
///         ("halt".to_owned(), "released", ResolutionSource::Absent),
///         ("new_checkout".to_owned(), "on", ResolutionSource::Literal),
///     ]
/// );
/// # Ok(())
/// # }
/// ```
pub struct Base {
    pub(crate) inner: Arc<BaseInner>,
}

impl Base {
    /// The catalog this base was built from.
    pub fn catalog(&self) -> &Catalog {
        &self.inner.catalog
    }

    /// The process kind the settings were resolved for.
    pub fn process_kind(&self) -> &ProcessKind {
        &self.inner.kind
    }

    /// The revision of every declaration's resolved variant: the digest, in
    /// key order, of each key and the variant it resolved to. A declaration
    /// this process kind does not own contributes its code default (a kill
    /// switch its safe state).
    pub fn revision(&self) -> Revision {
        self.inner.revision
    }

    /// The same revision over the declarations every process kind owns, the one
    /// set every kind resolves.
    pub fn shared_revision(&self) -> Revision {
        self.inner.shared_revision
    }

    /// What every declaration resolved to, in key order.
    pub fn resolved(&self) -> impl ExactSizeIterator<Item = Resolution<'_>> + '_ {
        let inner = &*self.inner;
        inner.catalog.by_key.values().map(move |&index| {
            let entry = &inner.catalog.entries[index];
            let (variant, source) = match inner.states[index] {
                BaseState::Flag(Some(variant)) => {
                    (entry.variants[variant], ResolutionSource::Literal)
                }
                BaseState::Flag(None) => (
                    entry.default_variant.unwrap_or_default(),
                    ResolutionSource::Absent,
                ),
                BaseState::Switch(Some(state)) => (state.as_str(), ResolutionSource::Literal),
                BaseState::Switch(None) => (
                    SwitchState::safe_for(entry).as_str(),
                    ResolutionSource::Absent,
                ),
            };
            Resolution {
                declared: Declared::new(&inner.catalog.prefix, entry),
                variant,
                source,
            }
        })
    }

    /// How many evaluations failed with `cause` on views of this base, whatever
    /// level their events were emitted at.
    pub fn error_count(&self, cause: ErrorCause) -> u64 {
        self.inner.tally.count(cause)
    }

    /// The scope a candidate is read for and a snapshot is validated against.
    pub fn validation_scope(&self) -> ValidationScope {
        ValidationScope::new(Arc::clone(&self.inner))
    }
}

impl fmt::Debug for Base {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Base")
            .field("process_kind", &self.inner.kind)
            .field("revision", &self.inner.revision)
            .finish_non_exhaustive()
    }
}

/// Whether a declaration's resolved variant came from a setting or from its
/// default.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResolutionSource {
    /// A setting supplied the value.
    Literal,
    /// No setting was present: the code default, or a kill switch's safe state.
    Absent,
}

/// One declaration's resolved variant and where it came from.
#[derive(Clone, Copy, Debug)]
pub struct Resolution<'a> {
    declared: Declared<'a>,
    variant: &'static str,
    source: ResolutionSource,
}

impl<'a> Resolution<'a> {
    /// The declaration's key.
    pub fn key(&self) -> &'a FlagKey {
        self.declared.key()
    }

    /// The declaration itself, so a reader of what resolved can ask what the
    /// declaration allows (its owners, its class, its kind) without looking the
    /// key up again.
    pub fn declared(&self) -> Declared<'a> {
        self.declared
    }

    /// The resolved variant: a flag value's variant name, or `engaged` or
    /// `released`.
    pub fn variant(&self) -> &'static str {
        self.variant
    }

    /// Whether a setting or the default supplied it.
    pub fn source(&self) -> ResolutionSource {
        self.source
    }
}

/// Why a literal or a requirement could not be recorded.
///
/// No variant carries the text it refused.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum BaseError {
    /// The process kind belongs to another catalog.
    #[error("the process kind belongs to another catalog")]
    ForeignKind,
    /// No declaration has that key.
    #[error("no declaration has that key")]
    UnknownKey,
    /// A flag literal named a kill switch, or a switch literal named a flag.
    #[error("the declaration is not of the kind the literal is for")]
    WrongKind,
    /// This process kind does not own the declaration, so no value may be
    /// presented for it.
    #[error("this process kind does not own the declaration")]
    NotOwned,
    /// The variant is not one the flag declares.
    #[error("the variant is not declared by the flag")]
    UndeclaredVariant,
    /// A literal was already recorded for the declaration.
    #[error("a literal was already recorded for the declaration")]
    RepeatedLiteral,
    /// The flag declares no required settings for that variant.
    #[error("the flag declares no required settings for that variant")]
    NoRequiredSettings,
}

/// Builds a [`Base`] from the settings a process resolved.
///
/// The builder takes the catalog by value, so a catalog is consumed exactly
/// once. Report each setting that was present with [`flag`](Self::flag) or
/// [`switch`](Self::switch), and each enabling variant whose required settings
/// were absent or empty with [`settings_absent`](Self::settings_absent); a
/// declaration with no report resolved to its default.
pub struct BaseBuilder {
    catalog: Catalog,
    kind: ProcessKind,
    states: Vec<BaseState>,
    settings_absent: Vec<Vec<&'static str>>,
}

impl BaseBuilder {
    /// Starts a base for `kind` over `catalog`.
    ///
    /// # Errors
    ///
    /// [`BaseError::ForeignKind`] when `kind` belongs to another catalog.
    pub fn new(catalog: Catalog, kind: ProcessKind) -> Result<Self, BaseError> {
        if kind.catalog() != catalog.id {
            return Err(BaseError::ForeignKind);
        }
        let states = catalog
            .entries
            .iter()
            .map(|entry| match entry.kind {
                Kind::Switch => BaseState::Switch(None),
                Kind::Release | Kind::Mode => BaseState::Flag(None),
            })
            .collect();
        let settings_absent = vec![Vec::new(); catalog.entries.len()];
        Ok(Self {
            catalog,
            kind,
            states,
            settings_absent,
        })
    }

    /// The catalog being resolved, so declarations can be read after it was
    /// handed over.
    pub fn catalog(&self) -> &Catalog {
        &self.catalog
    }

    /// The process kind being resolved.
    pub fn process_kind(&self) -> &ProcessKind {
        &self.kind
    }

    fn locate(&self, key: &FlagKey) -> Result<usize, BaseError> {
        self.catalog
            .by_key
            .get(key)
            .copied()
            .ok_or(BaseError::UnknownKey)
    }

    /// Records the variant a setting supplied for a release or mode flag.
    ///
    /// # Errors
    ///
    /// [`BaseError::UnknownKey`], [`BaseError::WrongKind`] for a kill switch,
    /// [`BaseError::NotOwned`], [`BaseError::UndeclaredVariant`] and
    /// [`BaseError::RepeatedLiteral`].
    pub fn flag(&mut self, key: &FlagKey, variant: &str) -> Result<(), BaseError> {
        let index = self.locate(key)?;
        let entry = &self.catalog.entries[index];
        if entry.kind == Kind::Switch {
            return Err(BaseError::WrongKind);
        }
        if !entry.owners.contains(&self.kind) {
            return Err(BaseError::NotOwned);
        }
        let chosen = entry
            .variants
            .iter()
            .position(|declared| *declared == variant)
            .ok_or(BaseError::UndeclaredVariant)?;
        match &mut self.states[index] {
            BaseState::Flag(literal @ None) => *literal = Some(chosen),
            _ => return Err(BaseError::RepeatedLiteral),
        }
        Ok(())
    }

    /// Records the state a setting supplied for a kill switch.
    ///
    /// # Errors
    ///
    /// [`BaseError::UnknownKey`], [`BaseError::WrongKind`] for a flag,
    /// [`BaseError::NotOwned`] and [`BaseError::RepeatedLiteral`].
    pub fn switch(&mut self, key: &FlagKey, state: SwitchState) -> Result<(), BaseError> {
        let index = self.locate(key)?;
        let entry = &self.catalog.entries[index];
        if entry.kind != Kind::Switch {
            return Err(BaseError::WrongKind);
        }
        if !entry.owners.contains(&self.kind) {
            return Err(BaseError::NotOwned);
        }
        match &mut self.states[index] {
            BaseState::Switch(literal @ None) => *literal = Some(state),
            _ => return Err(BaseError::RepeatedLiteral),
        }
        Ok(())
    }

    /// Records that the settings an enabling `variant` requires were absent or
    /// empty. A published entry that selects the variant is then held instead
    /// of served.
    ///
    /// # Errors
    ///
    /// [`BaseError::UnknownKey`], [`BaseError::WrongKind`] for a kill switch,
    /// [`BaseError::NotOwned`], [`BaseError::UndeclaredVariant`] and
    /// [`BaseError::NoRequiredSettings`].
    pub fn settings_absent(&mut self, key: &FlagKey, variant: &str) -> Result<(), BaseError> {
        let index = self.locate(key)?;
        let entry = &self.catalog.entries[index];
        if entry.kind == Kind::Switch {
            return Err(BaseError::WrongKind);
        }
        if !entry.owners.contains(&self.kind) {
            return Err(BaseError::NotOwned);
        }
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

    /// Finishes the base and computes its revisions.
    pub fn finish(self) -> Base {
        let (mut all, mut shared) = (Vec::new(), Vec::new());
        for (index, entry) in self.catalog.entries.iter().enumerate() {
            let variant = match self.states[index] {
                BaseState::Flag(Some(variant)) => entry.variants[variant],
                BaseState::Flag(None) => entry.default_variant.unwrap_or_default(),
                BaseState::Switch(Some(state)) => state.as_str(),
                BaseState::Switch(None) => SwitchState::safe_for(entry).as_str(),
            };
            all.push((entry.key.as_str(), variant));
            if matches!(entry.owners, OwnedOwners::All) {
                shared.push((entry.key.as_str(), variant));
            }
        }
        let (revision, shared_revision) = (Revision::compute(all), Revision::compute(shared));
        Base {
            inner: Arc::new(BaseInner {
                catalog: self.catalog,
                kind: self.kind,
                states: self.states.into_boxed_slice(),
                settings_absent: self.settings_absent.into_boxed_slice(),
                revision,
                shared_revision,
                tally: ErrorTally::default(),
            }),
        }
    }
}

impl fmt::Debug for BaseBuilder {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BaseBuilder")
            .field("process_kind", &self.kind)
            .finish_non_exhaustive()
    }
}
