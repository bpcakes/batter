//! Candidates: what a loader read, before anything has judged it.

use std::{fmt, sync::Arc};

use crate::{
    base::BaseInner,
    decision::SwitchState,
    digest::DeclarationFingerprint,
    handle::{Flag, KillSwitch},
    key::name_or_placeholder,
    snapshot::{Carried, EntryState, ForeignSnapshot, Provenance, Snapshot, ValidationScope},
    version::Version,
};

/// One stored row as read: the key, kind and value-type text, the version the
/// row was last written at and its state text exactly as the store returned it.
///
/// Nothing about a row is checked until [`Snapshot::validate`]: a candidate
/// may hold rows for keys no declaration has, rows of the wrong type and
/// states of an encoding this build does not know. An archived key is not a
/// row: a loader omits it, so it loads as absent.
///
/// ```
/// use batter_flags_core::{Version, low_level::CandidateRow};
///
/// let row = CandidateRow::new("new_checkout", "release", "toggle", Version::new(7), r#"{"encoding":1,"variant":"on"}"#);
/// assert_eq!(row.state_text(), r#"{"encoding":1,"variant":"on"}"#);
/// ```
#[derive(Clone)]
pub struct CandidateRow {
    pub(crate) key: Box<str>,
    pub(crate) kind: Box<str>,
    pub(crate) value_type: Box<str>,
    pub(crate) updated: Version,
    pub(crate) state: Box<str>,
    /// The declaration this row was written for, when its source records one.
    pub(crate) written_for: WrittenFor,
}

/// Which declaration a row was written for.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WrittenFor {
    /// The source does not say; the store's rows carry no fingerprint.
    Unrecorded,
    /// The declaration with this fingerprint.
    Declaration(DeclarationFingerprint),
}

impl CandidateRow {
    /// A row as read from a store or a file.
    pub fn new(
        key: &str,
        kind: &str,
        value_type: &str,
        updated_version: Version,
        state: &str,
    ) -> Self {
        Self {
            key: Box::from(key),
            kind: Box::from(kind),
            value_type: Box::from(value_type),
            updated: updated_version,
            state: Box::from(state),
            written_for: WrittenFor::Unrecorded,
        }
    }

    /// Records the fingerprint of the declaration this row was written for,
    /// as the last-good file does. A row whose fingerprint differs from the
    /// running declaration's is held with cause `entry_type_mismatch`, even
    /// when its kind and value type still match.
    ///
    /// ```
    /// use batter_flags_core::{
    ///     CatalogBuilder, CivilDate, Expiry, Flag, SettingsPrefix, Toggle, Version,
    ///     low_level::{BaseBuilder, Candidate, CandidateRow, HoldCause, Snapshot},
    /// };
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut catalog = CatalogBuilder::new(SettingsPrefix::new("FLAG_")?);
    /// let serving = catalog.process_kind("serving")?;
    /// catalog.environment_class("hosted")?;
    /// let expiry = Expiry::on(CivilDate::new(2027, 3, 31)?);
    /// let fast = catalog.release_flag("fast_path", Toggle::Off, expiry).declare()?;
    /// let other = catalog.release_flag("other_path", Toggle::Off, expiry).declare()?;
    /// let base = BaseBuilder::new(catalog.build()?, serving)?.finish();
    /// let scope = base.validation_scope();
    ///
    /// let at = Version::new(3);
    /// let other_declaration = base.catalog().declarations().find(|d| d.key() == other.key()).unwrap();
    /// let stale = CandidateRow::flag(&fast, Toggle::On, at).written_for(other_declaration.fingerprint());
    /// let snapshot = Snapshot::validate(&scope, Candidate::from_last_good_file(&scope).at(at).row(stale))?;
    /// assert!(snapshot.held(batter_flags_core::Freshness::Fresh).any(|(key, cause)| key.as_str() == "fast_path"
    ///     && cause == HoldCause::EntryTypeMismatch));
    /// # Ok(())
    /// # }
    /// ```
    pub fn written_for(mut self, fingerprint: DeclarationFingerprint) -> Self {
        self.written_for = WrittenFor::Declaration(fingerprint);
        self
    }

    /// The canonical row for a release or mode flag at `variant`, with the
    /// state text a writer produces at encoding 1.
    pub fn flag<F: Flag>(flag: &F, variant: F::Value, updated_version: Version) -> Self {
        let handle = flag.handle();
        Self::new(
            flag.key().as_str(),
            handle.kind().as_str(),
            handle.value_type().as_str(),
            updated_version,
            &format!(
                "{{\"encoding\":1,\"variant\":\"{}\"}}",
                crate::value::FlagValue::variant(variant)
            ),
        )
    }

    /// The canonical row for a kill switch in `state`, with the state text a
    /// writer produces at encoding 1.
    pub fn switch(switch: &KillSwitch, state: SwitchState, updated_version: Version) -> Self {
        let handle = switch.handle();
        Self::new(
            switch.key().as_str(),
            handle.kind().as_str(),
            handle.value_type().as_str(),
            updated_version,
            &format!("{{\"encoding\":1,\"switch\":\"{}\"}}", state.as_str()),
        )
    }

    /// The row's key as read.
    pub fn key(&self) -> &str {
        &self.key
    }

    /// The row's kind tag as read.
    pub fn kind(&self) -> &str {
        &self.kind
    }

    /// The row's value-type tag as read.
    pub fn value_type(&self) -> &str {
        &self.value_type
    }

    /// The version that last changed the row.
    pub fn updated_version(&self) -> Version {
        self.updated
    }

    /// The state text, as it will be decoded and digested.
    pub fn state_text(&self) -> &str {
        &self.state
    }
}

impl fmt::Debug for CandidateRow {
    /// Shows the key only when it is a valid name, and never the state.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CandidateRow")
            .field("key", &name_or_placeholder(&self.key))
            .field("updated", &self.updated)
            .finish_non_exhaustive()
    }
}

/// Everything one read of the control plane or the last-good file returned:
/// the version it was read at and its rows.
///
/// A candidate is read *for* one scope; validating it against another is a
/// [`SnapshotRejection::ForeignScope`](crate::low_level::SnapshotRejection).
/// Without a version it is structurally corrupt.
///
/// ```
/// use batter_flags_core::{
///     CatalogBuilder, SettingsPrefix, Version,
///     low_level::{BaseBuilder, Candidate, SnapshotRejection, Snapshot},
/// };
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let mut catalog = CatalogBuilder::new(SettingsPrefix::new("FLAG_")?);
/// let serving = catalog.process_kind("serving")?;
/// catalog.environment_class("hosted")?;
/// let base = BaseBuilder::new(catalog.build()?, serving)?.finish();
/// let scope = base.validation_scope();
///
/// let versionless = Candidate::new(&scope);
/// assert_eq!(Snapshot::validate(&scope, versionless).unwrap_err(), SnapshotRejection::MissingVersion);
/// assert!(Snapshot::validate(&scope, Candidate::new(&scope).at(Version::new(1))).is_ok());
/// # Ok(())
/// # }
/// ```
pub struct Candidate {
    pub(crate) base: Arc<BaseInner>,
    pub(crate) version: Option<Version>,
    pub(crate) provenance: Provenance,
    pub(crate) rows: Vec<CandidateRow>,
    /// Per declaration, the state a held kill switch keeps serving.
    pub(crate) carried: Vec<Option<Carried>>,
}

impl Candidate {
    /// An empty candidate read from the control plane for `scope`, with no
    /// version yet.
    pub fn new(scope: &ValidationScope) -> Self {
        Self::read_from(scope, Provenance::ControlPlane)
    }

    /// An empty candidate read from the last-good file for `scope`, with no
    /// version yet.
    ///
    /// A snapshot validated from it can only be the first publication: it
    /// never confirms or replaces a control-plane snapshot, and no state it
    /// serves is last-good again.
    pub fn from_last_good_file(scope: &ValidationScope) -> Self {
        Self::read_from(scope, Provenance::LastGoodFile)
    }

    fn read_from(scope: &ValidationScope, provenance: Provenance) -> Self {
        Self {
            base: Arc::clone(&scope.base),
            version: None,
            provenance,
            rows: Vec::new(),
            carried: vec![None; scope.base.catalog.entries.len()],
        }
    }

    /// Sets the version the candidate was read at.
    #[must_use]
    pub fn at(mut self, version: Version) -> Self {
        self.version = Some(version);
        self
    }

    /// Adds a row.
    #[must_use]
    pub fn row(mut self, row: CandidateRow) -> Self {
        self.rows.push(row);
        self
    }

    /// Adds every row of `rows`.
    #[must_use]
    pub fn rows(mut self, rows: impl IntoIterator<Item = CandidateRow>) -> Self {
        self.rows.extend(rows);
        self
    }

    /// Carries the kill-switch states `previous` served into this candidate,
    /// so that a switch whose row is held keeps serving its last published
    /// state instead of falling back to its safe state: holding an entry never
    /// releases an engaged switch.
    ///
    /// # Errors
    ///
    /// [`ForeignSnapshot`] when `previous` was validated for another base.
    pub fn carrying(mut self, previous: &Snapshot) -> Result<Self, ForeignSnapshot> {
        if !Arc::ptr_eq(&self.base, &previous.base) {
            return Err(ForeignSnapshot);
        }
        for (slot, state) in self.carried.iter_mut().zip(previous.entries.iter()) {
            *slot = match state {
                EntryState::Switch(state) => Some(Carried {
                    state: *state,
                    since: previous.version,
                    provenance: previous.provenance(),
                }),
                EntryState::Held { carried, .. } => *carried,
                EntryState::Absent | EntryState::Flag(_) => None,
            };
        }
        Ok(self)
    }
}

impl fmt::Debug for Candidate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Candidate")
            .field("version", &self.version)
            .field("rows", &self.rows.len())
            .finish_non_exhaustive()
    }
}
