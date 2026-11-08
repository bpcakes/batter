//! Snapshots: a candidate that was validated entry by entry.
//!
//! Validation rejects a whole candidate only for structural corruption: no
//! version, a repeated key, or a candidate read for another scope. Everything
//! else is decided per entry, because one bad row must never take the other
//! flags down with it.

use std::{collections::HashSet, fmt, sync::Arc};

use thiserror::Error;

use crate::{
    base::BaseInner,
    candidate::{Candidate, CandidateRow, WrittenFor},
    decision::SwitchState,
    declaration::{Entry, Kind},
    digest::{DigestRow, SnapshotDigest},
    encoding::{self, Decoded, Expect},
    key::FlagKey,
    version::Version,
};

/// What a candidate is validated against: the catalog, the process kind and
/// which enabling variants had their required settings absent.
///
/// Taken from a [`Base`](crate::low_level::Base), so a scope cannot name a
/// catalog that no base resolved.
#[derive(Clone)]
pub struct ValidationScope {
    pub(crate) base: Arc<BaseInner>,
}

impl ValidationScope {
    pub(crate) fn new(base: Arc<BaseInner>) -> Self {
        Self { base }
    }
}

impl fmt::Debug for ValidationScope {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ValidationScope")
            .field("process_kind", &self.base.kind)
            .finish_non_exhaustive()
    }
}

/// Why an entry is held: its row is kept out of the served rule set.
///
/// A held release or mode flag serves the declared base; a held kill switch
/// keeps the state it last published, else its safe state. When several causes
/// apply the first in this order is reported: [`EnvironmentRestricted`],
/// [`EntryTypeMismatch`], [`UnknownRowEncoding`], [`UndeclaredVariant`],
/// [`RequiresSettingsAbsent`].
///
/// [`EnvironmentRestricted`]: Self::EnvironmentRestricted
/// [`EntryTypeMismatch`]: Self::EntryTypeMismatch
/// [`UnknownRowEncoding`]: Self::UnknownRowEncoding
/// [`UndeclaredVariant`]: Self::UndeclaredVariant
/// [`RequiresSettingsAbsent`]: Self::RequiresSettingsAbsent
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum HoldCause {
    /// The row's kind or value type is not the declaration's.
    EntryTypeMismatch,
    /// The row's state is not exactly encoding 1: a higher encoding, an extra
    /// field or a field of the wrong shape.
    UnknownRowEncoding,
    /// The row selects a variant the declaration does not declare.
    UndeclaredVariant,
    /// The row selects a variant whose required settings were absent or empty
    /// in this process when it resolved.
    RequiresSettingsAbsent,
    /// The declaration is restricted with `only_in`, so the control plane never
    /// supplies it, in any environment class.
    EnvironmentRestricted,
}

impl HoldCause {
    /// The cause's name in load events and heartbeats.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::EntryTypeMismatch => "entry_type_mismatch",
            Self::UnknownRowEncoding => "unknown_row_encoding",
            Self::UndeclaredVariant => "undeclared_variant",
            Self::RequiresSettingsAbsent => "requires_settings_absent",
            Self::EnvironmentRestricted => "environment_restricted",
        }
    }
}

/// Why a whole candidate was rejected: structural corruption only.
///
/// No variant carries text from the candidate.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum SnapshotRejection {
    /// The candidate carries no version.
    #[error("the candidate carries no version")]
    MissingVersion,
    /// A row repeats the key of an earlier row.
    #[error("row {row} repeats the key of an earlier row")]
    DuplicateKey {
        /// The position of the repeating row.
        row: usize,
    },
    /// The candidate was read for another scope.
    #[error("the candidate was read for another scope")]
    ForeignScope,
}

/// A snapshot or a view was offered to a base it does not belong to.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
#[error("the snapshot was validated for another base")]
pub struct ForeignSnapshot;

/// Where a candidate was read from.
///
/// The two sources have different standing. The control plane is the
/// authority; the last-good file is a stand-in read once at startup, which
/// confirms nothing and is never an applied version. A snapshot remembers its
/// provenance, and so does every kill-switch state it hands on, so a rule that
/// depends on where a state came from can be decided later.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Provenance {
    /// A control-plane read.
    ControlPlane,
    /// The last-good file.
    LastGoodFile,
}

impl Provenance {
    /// The provenance's name in diagnostics.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ControlPlane => "control_plane",
            Self::LastGoodFile => "last_good_file",
        }
    }
}

/// What a kill switch kept serving when its row was held.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Carried {
    pub(crate) state: SwitchState,
    pub(crate) since: Version,
    pub(crate) provenance: Provenance,
}

/// A kill switch state a snapshot keeps serving because its row is held.
///
/// Holding never releases an engaged switch, so the state outlives the
/// snapshot that published it; this says which state, since which version and
/// where it was first read from. Persistence needs the last: only a state read
/// from the control plane is last-good.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CarriedSwitch<'a> {
    key: &'a FlagKey,
    state: SwitchState,
    since: Version,
    provenance: Provenance,
}

impl<'a> CarriedSwitch<'a> {
    /// The kill switch.
    pub fn key(&self) -> &'a FlagKey {
        self.key
    }

    /// The state it keeps serving.
    pub fn state(&self) -> SwitchState {
        self.state
    }

    /// The version of the snapshot that last served the state as published.
    pub fn since(&self) -> Version {
        self.since
    }

    /// Where the state was first read from, however many holds ago.
    pub fn provenance(&self) -> Provenance {
        self.provenance
    }
}

/// One declaration's state in a snapshot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum EntryState {
    /// No row, or a row for another process kind.
    Absent,
    /// A published variant: an index into the declaration's variants.
    Flag(usize),
    /// A published kill-switch state.
    Switch(SwitchState),
    /// A row that is kept out of the served rule set.
    Held {
        cause: HoldCause,
        carried: Option<Carried>,
    },
}

/// A candidate validated against a scope: per declaration, what the control
/// plane published for this process kind, what is held and why.
///
/// A snapshot is immutable and cheap to share. Its [`digest`](Self::digest)
/// covers the candidate exactly as read, held rows included, so a read that
/// returns the same version and digest confirms the snapshot already served.
///
/// ```
/// use batter_flags_core::{
///     CatalogBuilder, CivilDate, Expiry, Flag, SettingsPrefix, Toggle, Version,
///     low_level::{BaseBuilder, Candidate, CandidateRow, Snapshot},
/// };
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let mut catalog = CatalogBuilder::new(SettingsPrefix::new("FLAG_")?);
/// let serving = catalog.process_kind("serving")?;
/// catalog.environment_class("hosted")?;
/// let expiry = Expiry::on(CivilDate::new(2027, 3, 31)?);
/// let new_checkout = catalog.release_flag("new_checkout", Toggle::Off, expiry).declare()?;
/// let base = BaseBuilder::new(catalog.build()?, serving)?.finish();
///
/// let scope = base.validation_scope();
/// let candidate = Candidate::new(&scope)
///     .at(Version::new(7))
///     .row(CandidateRow::flag(&new_checkout, Toggle::On, Version::new(7)))
///     .row(CandidateRow::new("retired_flag", "mode", "toggle", Version::new(3), r#"{"encoding":1,"variant":"on"}"#));
/// let snapshot = Snapshot::validate(&scope, candidate)?;
///
/// assert_eq!(snapshot.version(), Version::new(7));
/// assert_eq!(snapshot.unknown(), 1); // a key no declaration has is counted, never held
/// assert_eq!(snapshot.held().count(), 0);
/// # Ok(())
/// # }
/// ```
pub struct Snapshot {
    pub(crate) base: Arc<BaseInner>,
    pub(crate) version: Version,
    provenance: Provenance,
    digest: SnapshotDigest,
    /// Per declaration, in declaration order.
    pub(crate) entries: Box<[EntryState]>,
    unknown: usize,
    not_owned: usize,
}

impl Snapshot {
    /// Validates `candidate` against `scope`.
    ///
    /// A row is ignored when it names no declaration (counted as
    /// [`unknown`](Self::unknown)) or names a declaration this process kind
    /// does not own (counted as [`not_owned`](Self::not_owned)); neither is
    /// held, and validation emits no event. Any other row is either published
    /// or held for one [`HoldCause`].
    ///
    /// # Errors
    ///
    /// [`SnapshotRejection`] for a candidate that is structurally corrupt.
    pub fn validate(
        scope: &ValidationScope,
        candidate: Candidate,
    ) -> Result<Self, SnapshotRejection> {
        let Candidate {
            base,
            version,
            provenance,
            rows,
            carried,
        } = candidate;
        if !Arc::ptr_eq(&base, &scope.base) {
            return Err(SnapshotRejection::ForeignScope);
        }
        let version = version.ok_or(SnapshotRejection::MissingVersion)?;
        let mut keys = HashSet::with_capacity(rows.len());
        if let Some(row) = rows.iter().position(|row| !keys.insert(&*row.key)) {
            return Err(SnapshotRejection::DuplicateKey { row });
        }
        let digest = SnapshotDigest::compute(
            version,
            rows.iter()
                .map(|row| DigestRow {
                    key: &row.key,
                    kind: &row.kind,
                    value_type: &row.value_type,
                    updated: row.updated,
                    state: &row.state,
                })
                .collect(),
        );
        let catalog = &base.catalog;
        let mut entries = vec![EntryState::Absent; catalog.entries.len()];
        let (mut unknown, mut not_owned) = (0, 0);
        for row in &rows {
            let index = FlagKey::new(&row.key)
                .ok()
                .and_then(|key| catalog.by_key.get(&key).copied());
            let Some(index) = index else {
                unknown += 1;
                continue;
            };
            let entry = &catalog.entries[index];
            if !entry.owners.contains(&base.kind) {
                not_owned += 1;
                continue;
            }
            entries[index] = classify(&base, index, entry, row, carried[index]);
        }
        Ok(Self {
            base,
            version,
            provenance,
            digest,
            entries: entries.into_boxed_slice(),
            unknown,
            not_owned,
        })
    }

    /// The version the candidate was read at.
    pub fn version(&self) -> Version {
        self.version
    }

    /// Where the candidate was read from.
    pub fn provenance(&self) -> Provenance {
        self.provenance
    }

    /// The digest of the candidate exactly as read, held rows included.
    pub fn digest(&self) -> SnapshotDigest {
        self.digest
    }

    /// How many rows named no declaration.
    pub fn unknown(&self) -> usize {
        self.unknown
    }

    /// How many rows named a declaration this process kind does not own.
    pub fn not_owned(&self) -> usize {
        self.not_owned
    }

    /// The kill switches whose held rows keep serving a state they last
    /// published, in declaration order.
    pub fn carried(&self) -> impl Iterator<Item = CarriedSwitch<'_>> + '_ {
        self.entries
            .iter()
            .zip(self.base.catalog.entries.iter())
            .filter_map(|(state, entry)| match state {
                EntryState::Held {
                    carried: Some(kept),
                    ..
                } => Some(CarriedSwitch {
                    key: &entry.key,
                    state: kept.state,
                    since: kept.since,
                    provenance: kept.provenance,
                }),
                _ => None,
            })
    }

    /// The held entries, in declaration order, with why each is held.
    pub fn held(&self) -> impl Iterator<Item = (&FlagKey, HoldCause)> + '_ {
        self.entries
            .iter()
            .zip(self.base.catalog.entries.iter())
            .filter_map(|(state, entry)| match state {
                EntryState::Held { cause, .. } => Some((&entry.key, *cause)),
                _ => None,
            })
    }
}

impl fmt::Debug for Snapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Snapshot")
            .field("version", &self.version)
            .field("provenance", &self.provenance)
            .field("digest", &self.digest)
            .field("unknown", &self.unknown)
            .field("not_owned", &self.not_owned)
            .finish_non_exhaustive()
    }
}

/// Decides what one owned row is: published or held.
fn classify(
    base: &BaseInner,
    index: usize,
    entry: &Entry,
    row: &CandidateRow,
    carried: Option<Carried>,
) -> EntryState {
    let held = |cause| EntryState::Held {
        cause,
        carried: carried.filter(|_| entry.kind == Kind::Switch),
    };
    if entry.only_in.is_some() {
        return held(HoldCause::EnvironmentRestricted);
    }
    if *row.kind != *entry.kind.as_str() || *row.value_type != *entry.value_type.as_str() {
        return held(HoldCause::EntryTypeMismatch);
    }
    match row.written_for {
        WrittenFor::Declaration(fingerprint) if fingerprint != entry.fingerprint => {
            return held(HoldCause::EntryTypeMismatch);
        }
        WrittenFor::Declaration(_) | WrittenFor::Unrecorded => {}
    }
    let expect = match entry.kind {
        Kind::Switch => Expect::Switch,
        Kind::Release | Kind::Mode => Expect::Flag,
    };
    match encoding::decode(&row.state, expect) {
        Err(_) => held(HoldCause::UnknownRowEncoding),
        Ok(Decoded::Switch(state)) => EntryState::Switch(state),
        Ok(Decoded::Flag(variant)) => {
            let Some(chosen) = entry
                .variants
                .iter()
                .position(|declared| **declared == *variant)
            else {
                return held(HoldCause::UndeclaredVariant);
            };
            if base.settings_absent[index].contains(&entry.variants[chosen]) {
                held(HoldCause::RequiresSettingsAbsent)
            } else {
                EntryState::Flag(chosen)
            }
        }
    }
}
