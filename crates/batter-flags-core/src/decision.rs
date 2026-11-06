//! Decisions: what a view serves, why, and from where.

use std::fmt;

use crate::{
    declaration::{Entry, SafeState},
    digest::Revision,
    key::FlagKey,
    pin::PinOrigin,
    version::Version,
};

/// The state a kill switch serves.
///
/// ```
/// use batter_flags_core::{SafeState, SwitchState};
///
/// assert_eq!(SwitchState::from(SafeState::Engaged), SwitchState::Engaged);
/// assert_eq!(SwitchState::Released.as_str(), "released");
/// assert!(SwitchState::Engaged.is_engaged());
/// ```
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(rename_all = "snake_case")
)]
pub enum SwitchState {
    /// The switch is tripped: what it gates is disabled.
    Engaged,
    /// The switch is not tripped.
    Released,
}

impl SwitchState {
    /// The state's wire name, as stored rows and decisions spell it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Engaged => "engaged",
            Self::Released => "released",
        }
    }

    /// Whether the switch is tripped.
    pub fn is_engaged(self) -> bool {
        self == Self::Engaged
    }

    /// The state `entry` serves where nothing supplies one: its safe state if
    /// it is a kill switch.
    pub(crate) fn safe_for(entry: &Entry) -> Self {
        entry.safe_state.map_or(Self::Released, Self::from)
    }
}

impl From<SafeState> for SwitchState {
    fn from(safe: SafeState) -> Self {
        match safe {
            SafeState::Engaged => Self::Engaged,
            SafeState::Released => Self::Released,
        }
    }
}

/// Why a decision has its value, in the OpenTelemetry semantic-convention
/// spelling of `feature_flag.result.reason`.
///
/// A reason is a pure function of the source, the layer and the view's
/// freshness and load policy. Targeting and percentage reasons are not here:
/// no decision is targeted.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Reason {
    /// A published or declared value was served as it stands.
    Static,
    /// No layer supplied a value, so the declared base was served.
    Default,
    /// A governing kill switch is engaged: the flag serves its code default.
    Disabled,
    /// A pinned decision was replayed.
    Cached,
    /// The value is old: the view is stale, the source is a persisted file, or
    /// a held kill switch kept its last published state.
    Stale,
    /// The evaluation could not be answered; the decision carries its
    /// [`ErrorCause`].
    Error,
}

impl Reason {
    /// The semantic-convention value of `feature_flag.result.reason`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Static => "static",
            Self::Default => "default",
            Self::Disabled => "disabled",
            Self::Cached => "cached",
            Self::Stale => "stale",
            Self::Error => "error",
        }
    }
}

impl fmt::Display for Reason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Whether a view's snapshot has been confirmed within the loader's maximum
/// age, computed once by whoever published the view.
///
/// This crate reads no clock: [`Stale`](Self::Stale) is what a publisher
/// asserted at capture. A stale view applies each kill switch's
/// [`StaleRule`](crate::StaleRule).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Freshness {
    /// A control-plane read confirmed the snapshot within the maximum age.
    Fresh,
    /// No read has confirmed the snapshot within the maximum age.
    Stale,
}

/// How a view captured before the first publication treats its decisions.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PrePublication {
    /// The process serves its declared base until something is published.
    ServeDefaults,
    /// The process is not ready: decisions report `provider_not_ready`.
    Unready,
}

/// The key of a kill switch, as a layer and a pin resolution name it.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct KillSwitchKey(FlagKey);

impl KillSwitchKey {
    pub(crate) fn new(key: FlagKey) -> Self {
        Self(key)
    }

    /// The switch's key.
    pub fn as_key(&self) -> &FlagKey {
        &self.0
    }

    /// The switch's key text.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Display for KillSwitchKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.0.as_str())
    }
}

/// The layer of a view that supplied a decision's value.
///
/// Release and mode flags layer a published entry over the declared base; a
/// kill switch layers its settings floor, the stale rule and its served state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Layer {
    /// A declared setting or a published entry supplied the value.
    Entry,
    /// The declared base supplied it: the code default, or a setting's
    /// literal while a control plane is the source.
    Base,
    /// A held kill switch kept the state it last published.
    Held {
        /// The version of the snapshot that last served the state as
        /// published, not the version its row was last written at.
        since: Version,
    },
    /// The kill switch's settings floor engaged it.
    Floor,
    /// The view is stale and the switch's rule engaged it.
    StaleRule,
    /// A governing kill switch is engaged, so the flag serves its code default.
    Override {
        /// The engaged governing switch.
        by: KillSwitchKey,
    },
}

/// Where a decision's information came from.
///
/// Every decision under a loaded view reports the view's version whichever
/// layer supplied the value. A source is whatever the view's publisher
/// asserted; this crate cannot tell the batter-flags reader from any other
/// caller of [`low_level`](crate::low_level).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Source {
    /// Declared mode: the settings read at startup.
    Declared {
        /// The revision of what the process resolved.
        revision: Revision,
    },
    /// A control plane is configured but nothing is published yet.
    NotLoaded {
        /// The revision of what the process resolved.
        revision: Revision,
    },
    /// A snapshot loaded from the control plane.
    ControlPlane {
        /// The snapshot's version.
        version: Version,
        /// Whether a read confirmed it recently.
        freshness: Freshness,
    },
    /// A snapshot restored from the last-good file.
    PersistedFile {
        /// The snapshot's version.
        version: Version,
    },
    /// A pinned decision, replayed.
    Pinned {
        /// Where the pin was taken. It is data the application stored, never a
        /// control-plane fact.
        origin: PinOrigin,
    },
    /// Injected test flags.
    Test {
        /// The injected version.
        version: Version,
        /// The injected freshness.
        freshness: Freshness,
    },
}

/// A source's version or revision, as `feature_flag.version` spells it.
#[derive(Clone, Copy, Debug)]
pub(crate) enum SourceVersion {
    Version(Version),
    Revision(Revision),
}

impl fmt::Display for SourceVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Version(version) => version.fmt(formatter),
            Self::Revision(revision) => revision.fmt(formatter),
        }
    }
}

impl Source {
    /// The version or revision `feature_flag.version` reports; for a pin, its
    /// origin's.
    pub(crate) fn version_text(&self) -> SourceVersion {
        match *self {
            Self::Declared { revision } | Self::NotLoaded { revision } => {
                SourceVersion::Revision(revision)
            }
            Self::ControlPlane { version, .. }
            | Self::PersistedFile { version }
            | Self::Test { version, .. } => SourceVersion::Version(version),
            Self::Pinned { origin } => origin.version_text(),
        }
    }
}

/// Why an evaluation could not be answered. Every decision whose reason is
/// [`Reason::Error`] carries one, and no other decision does.
///
/// The causes after [`ProviderNotReady`](Self::ProviderNotReady) describe a
/// refused pin ([`PinRejection`](crate::PinRejection)); a [`Decision`] never
/// carries them, but their events and counts use the same closed vocabulary.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ErrorCause {
    /// The handle belongs to another catalog.
    ForeignCatalog,
    /// This process kind does not own the declaration.
    NotOwned,
    /// A control plane is configured, nothing is published and the load policy
    /// withholds readiness.
    ProviderNotReady,
    /// A pin's envelope version is not one this build reads.
    PinUnsupportedVersion,
    /// A pin's shape is invalid.
    PinMalformed,
    /// A pin names another flag.
    PinKeyMismatch,
    /// A pin was taken for another kind of declaration.
    PinKindChanged,
    /// A pin was taken for another value type.
    PinValueTypeChanged,
    /// A pin's variant is not declared by the running flag.
    PinUnknownVariant,
    /// A test pin met a view that is not a test view.
    PinTestProvenance,
}

impl ErrorCause {
    /// Every cause, in declaration order.
    pub const ALL: [Self; 10] = [
        Self::ForeignCatalog,
        Self::NotOwned,
        Self::ProviderNotReady,
        Self::PinUnsupportedVersion,
        Self::PinMalformed,
        Self::PinKeyMismatch,
        Self::PinKindChanged,
        Self::PinValueTypeChanged,
        Self::PinUnknownVariant,
        Self::PinTestProvenance,
    ];

    /// The semantic-convention `error.type` of an evaluation with this cause.
    pub fn error_type(self) -> &'static str {
        match self {
            Self::ForeignCatalog | Self::NotOwned => "flag_not_found",
            Self::ProviderNotReady => "provider_not_ready",
            Self::PinKeyMismatch | Self::PinKindChanged | Self::PinValueTypeChanged => {
                "type_mismatch"
            }
            Self::PinUnsupportedVersion | Self::PinMalformed | Self::PinUnknownVariant => {
                "parse_error"
            }
            Self::PinTestProvenance => "general",
        }
    }

    /// The `batter.flags.cause` an event adds beside `flag_not_found`:
    /// `foreign_catalog` or `not_owned`. Other causes add none.
    pub fn handle_cause(self) -> Option<&'static str> {
        match self {
            Self::ForeignCatalog => Some("foreign_catalog"),
            Self::NotOwned => Some("not_owned"),
            _ => None,
        }
    }

    pub(crate) fn index(self) -> usize {
        self as usize
    }
}

/// What a view served for one evaluation, and why.
///
/// Fields are private and a decision has no public constructor: only a view
/// builds one, so a value, its variant name, its reason, its layer and its
/// source cannot disagree.
///
/// ```compile_fail,E0451
/// use batter_flags_core::{Decision, SwitchState};
///
/// let _decision = Decision::<SwitchState> {
///     value: todo!(),
///     variant: todo!(),
///     reason: todo!(),
///     source: todo!(),
///     layer: todo!(),
///     error: todo!(),
/// };
/// ```
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Decision<T> {
    value: T,
    variant: &'static str,
    reason: Reason,
    source: Source,
    layer: Layer,
    error: Option<ErrorCause>,
}

impl<T> Decision<T> {
    /// A decision some layer answered.
    pub(crate) fn served(
        (value, variant): (T, &'static str),
        reason: Reason,
        source: Source,
        layer: Layer,
    ) -> Self {
        debug_assert!(reason != Reason::Error, "an error carries its cause");
        Self {
            value,
            variant,
            reason,
            source,
            layer,
            error: None,
        }
    }

    /// A decision that could not be answered: the value is the fallback.
    pub(crate) fn failed(
        (value, variant): (T, &'static str),
        cause: ErrorCause,
        source: Source,
        layer: Layer,
    ) -> Self {
        Self {
            value,
            variant,
            reason: Reason::Error,
            source,
            layer,
            error: Some(cause),
        }
    }

    /// The variant name of the value: a flag value's variant, or `engaged` or
    /// `released`.
    pub fn variant(&self) -> &'static str {
        self.variant
    }

    /// Why the decision has its value.
    pub fn reason(&self) -> Reason {
        self.reason
    }

    /// Where the decision's information came from.
    pub fn source(&self) -> Source {
        self.source
    }

    /// The layer that supplied the value.
    pub fn layer(&self) -> &Layer {
        &self.layer
    }

    /// The cause of an error decision, and only of one: `Some` exactly when
    /// the reason is [`Reason::Error`].
    pub fn error(&self) -> Option<ErrorCause> {
        self.error
    }
}

impl<T: Copy> Decision<T> {
    /// The decided value.
    pub fn value(&self) -> T {
        self.value
    }
}
