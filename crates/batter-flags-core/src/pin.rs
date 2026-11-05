//! Pinned decisions: an admission-time decision, stored by the application
//! and replayed on retry.
//!
//! Batter never writes a pin. The application takes one from a view with
//! [`SnapshotView::pin`](crate::SnapshotView::pin), stores it in its own row,
//! and replays it with
//! [`SnapshotView::resolve_pinned`](crate::SnapshotView::resolve_pinned). A
//! stored pin is untrusted application data: it is validated against the
//! running declaration at every replay, and its origin stays data, never a
//! control-plane fact.

#[cfg(feature = "serde")]
mod wire;

use std::{fmt, marker::PhantomData};

use thiserror::Error;

use crate::{
    decision::{Decision, ErrorCause, KillSwitchKey, Source, SourceVersion},
    digest::Revision,
    handle::{Flag, Handle, ModeFlag, ReleaseFlag},
    key::name_or_placeholder,
    value::FlagValue,
    version::Version,
    view::SnapshotView,
};

/// Where a pin was taken: the source of the view it was taken from.
///
/// An origin is data the application stored. A replay compares nothing against
/// it except that a [`Test`](Self::Test) origin only replays under a test view.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PinOrigin {
    /// Taken in declared mode.
    Declared {
        /// The revision of what the process resolved.
        revision: Revision,
    },
    /// Taken before the first publication.
    NotLoaded {
        /// The revision of what the process resolved.
        revision: Revision,
    },
    /// Taken from a control-plane snapshot.
    ControlPlane {
        /// The snapshot's version.
        version: Version,
    },
    /// Taken from a snapshot restored from the last-good file.
    PersistedFile {
        /// The snapshot's version.
        version: Version,
    },
    /// Taken from injected test flags.
    Test {
        /// The injected version.
        version: Version,
    },
}

impl PinOrigin {
    /// The origin's class name, as a stored pin spells it.
    pub fn class(self) -> &'static str {
        match self {
            Self::Declared { .. } => "declared",
            Self::NotLoaded { .. } => "not_loaded",
            Self::ControlPlane { .. } => "control_plane",
            Self::PersistedFile { .. } => "persisted_file",
            Self::Test { .. } => "test",
        }
    }

    /// The origin of a pin taken from a view with `source`.
    pub(crate) fn of(source: Source) -> Self {
        match source {
            Source::Declared { revision } => Self::Declared { revision },
            Source::NotLoaded { revision } => Self::NotLoaded { revision },
            Source::ControlPlane { version, .. } => Self::ControlPlane { version },
            Source::PersistedFile { version } => Self::PersistedFile { version },
            Source::Test { version, .. } => Self::Test { version },
            Source::Pinned { origin } => origin,
        }
    }

    pub(crate) fn version_text(self) -> SourceVersion {
        match self {
            Self::Declared { revision } | Self::NotLoaded { revision } => {
                SourceVersion::Revision(revision)
            }
            Self::ControlPlane { version }
            | Self::PersistedFile { version }
            | Self::Test { version } => SourceVersion::Version(version),
        }
    }
}

/// Why a stored pin cannot be replayed against the running declaration.
///
/// No variant carries text from the stored pin.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum PinRejection {
    /// The flag handle belongs to another catalog.
    #[error("the flag handle belongs to another catalog")]
    ForeignHandle,
    /// This process kind neither owns the flag nor is listed in its `pinned_by`.
    #[error("this process kind neither owns the flag nor may resolve its pins")]
    NotOwned,
    /// The pin's envelope version is not one this build reads.
    #[error("the pin's envelope version {v} is not supported")]
    UnsupportedVersion {
        /// The version the pin carries.
        v: u64,
    },
    /// The pin's shape is invalid, such as an origin whose fields disagree with
    /// its class.
    #[error("the pin is malformed")]
    Malformed,
    /// The pin names another flag.
    #[error("the pin names another flag")]
    KeyMismatch,
    /// The pin was taken for another kind of declaration.
    #[error("the pin was taken for another kind of declaration")]
    KindChanged,
    /// The pin was taken for another value type.
    #[error("the pin was taken for another value type")]
    ValueTypeChanged,
    /// The pin's variant is not declared by the running flag, such as a
    /// variant a newer build added.
    #[error("the pin's variant is not declared by the running flag")]
    UnknownVariant,
    /// A pin taken from test flags met a view that is not a test view.
    #[error("a test pin met a view that is not a test view")]
    TestProvenance,
}

impl PinRejection {
    /// The closed cause this rejection's event and count use.
    pub fn cause(self) -> ErrorCause {
        match self {
            Self::ForeignHandle => ErrorCause::ForeignCatalog,
            Self::NotOwned => ErrorCause::NotOwned,
            Self::UnsupportedVersion { .. } => ErrorCause::PinUnsupportedVersion,
            Self::Malformed => ErrorCause::PinMalformed,
            Self::KeyMismatch => ErrorCause::PinKeyMismatch,
            Self::KindChanged => ErrorCause::PinKindChanged,
            Self::ValueTypeChanged => ErrorCause::PinValueTypeChanged,
            Self::UnknownVariant => ErrorCause::PinUnknownVariant,
            Self::TestProvenance => ErrorCause::PinTestProvenance,
        }
    }

    /// The semantic-convention `error.type` of the rejection's event.
    pub fn error_type(self) -> &'static str {
        self.cause().error_type()
    }
}

/// The outcome of replaying a stored pin.
///
/// The checks run in a fixed order: the handle, then the pin against the
/// running declaration, then the governing kill switch. A mismatched pin is
/// therefore never reported as [`Overridden`](Self::Overridden).
#[derive(Debug)]
#[must_use = "a pin resolution says whether the pinned decision may run"]
pub enum PinnedResolution<'v, T: FlagValue> {
    /// The pin cannot be replayed. The [`PinRefusal`] is the only way to read
    /// the flag live instead; dropping it defers.
    Incompatible(PinRefusal<'v, T>),
    /// The flag's governing kill switch is engaged in this view, so the
    /// decision is the code default with [`Layer::Override`](crate::Layer).
    Overridden {
        /// The code default, with reason `disabled`.
        decision: Decision<T>,
        /// The engaged governing switch.
        by: KillSwitchKey,
    },
    /// The pinned variant, with source [`Source::Pinned`] and reason `cached`.
    Honoured(Decision<T>),
}

impl<T: FlagValue> PinnedResolution<'_, T> {
    /// Why the pin was refused, if it was.
    pub fn rejection(&self) -> Option<PinRejection> {
        match self {
            Self::Incompatible(refusal) => Some(refusal.rejection()),
            Self::Overridden { .. } | Self::Honoured(_) => None,
        }
    }
}

/// A stored pin that cannot be replayed, and the one way to act on that.
///
/// Only [`SnapshotView::resolve_pinned`] makes one, and only for a pin it
/// refused. It is bound to that call's view, which it borrows, and to that
/// flag, so a fallback cannot be taken without a refusal, for another flag or
/// on another invocation's view. Taking the fallback consumes it.
///
/// A caller has exactly two ways forward. Reading the flag live from the same
/// view is [`fall_back_to_live`](Self::fall_back_to_live). Deferring is not
/// reading: the caller reports [`rejection`](Self::rejection) through its own
/// error path and drops the refusal. A scheduled job with no admission-time
/// decision, or a row that predates pinning, never has a refusal: those are
/// facts only the application knows, so it reads live through its own explicit
/// path and says why there.
///
/// ```
/// use batter_flags_core::{
///     CatalogBuilder, CivilDate, Expiry, Pinned, PinnedResolution, PinRejection, SettingsPrefix,
///     Toggle,
///     low_level::BaseBuilder,
/// };
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let mut catalog = CatalogBuilder::new(SettingsPrefix::new("FLAG_")?);
/// let serving = catalog.process_kind("serving")?;
/// catalog.environment_class("hosted")?;
/// let expiry = Expiry::on(CivilDate::new(2027, 3, 31)?);
/// let fast_path = catalog.release_flag("fast_path", Toggle::Off, expiry).declare()?;
/// let new_checkout = catalog.release_flag("new_checkout", Toggle::Off, expiry).declare()?;
/// let base = BaseBuilder::new(catalog.build()?, serving)?.finish();
/// let view = base.declared_view();
///
/// // A pin of another flag of the same type cannot be replayed here.
/// let wrong_row: Pinned<Toggle> = view.pin(&fast_path);
/// match view.resolve_pinned(&new_checkout, &wrong_row) {
///     PinnedResolution::Incompatible(refusal) => {
///         assert_eq!(refusal.rejection(), PinRejection::KeyMismatch);
///         // The caller chose to read live; this is a separate evaluation.
///         assert_eq!(refusal.fall_back_to_live().value(), Toggle::Off);
///     }
///     _ => unreachable!("the pin names another flag"),
/// }
/// # Ok(())
/// # }
/// ```
///
/// A refusal cannot be built, so the fallback cannot be reached without one:
///
/// ```compile_fail,E0451
/// use batter_flags_core::{PinRefusal, PinRejection, Toggle};
///
/// fn forge<'v>() -> PinRefusal<'v, Toggle> {
///     PinRefusal {
///         view: todo!(),
///         handle: todo!(),
///         default: todo!(),
///         rejection: PinRejection::Malformed,
///     }
/// }
/// ```
#[must_use = "a refused pin must be answered: read live or defer"]
pub struct PinRefusal<'v, T: FlagValue> {
    view: &'v SnapshotView,
    handle: Handle,
    default: T,
    rejection: PinRejection,
}

impl<'v, T: FlagValue> PinRefusal<'v, T> {
    pub(crate) fn new(
        view: &'v SnapshotView,
        handle: Handle,
        default: T,
        rejection: PinRejection,
    ) -> Self {
        Self {
            view,
            handle,
            default,
            rejection,
        }
    }

    /// Why the pin was refused.
    pub fn rejection(&self) -> PinRejection {
        self.rejection
    }

    /// Reads the flag live from the view that refused the pin, as
    /// [`SnapshotView::evaluate`] does, and emits that evaluation's own event.
    ///
    /// The refused pin's event is already emitted and carries no variant, so
    /// the two are separate evaluations. A kind that may only resolve the
    /// flag's pins, not own it, gets the `not_owned` error decision a live read
    /// of it always gets.
    pub fn fall_back_to_live(self) -> Decision<T> {
        self.view.evaluate_handle(&self.handle, self.default)
    }
}

impl<T: FlagValue> fmt::Debug for PinRefusal<'_, T> {
    /// Shows the flag's key and the arm, never anything from the stored pin.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PinRefusal")
            .field("flag", &self.handle.key().as_str())
            .field("rejection", &self.rejection)
            .finish()
    }
}

/// A flag whose decision can be pinned: a release flag or a mode flag.
///
/// Implemented only by [`ReleaseFlag`] and [`ModeFlag`], and sealed with
/// [`Flag`]: a kill switch acts through gates and is never pinned.
pub trait Pinnable: Flag {}

impl<T: FlagValue> Pinnable for ReleaseFlag<T> {}
impl<T: FlagValue> Pinnable for ModeFlag<T> {}

/// A decision pinned for replay: the flag's key, kind, value type, the variant
/// that was served and the origin it was taken from.
///
/// The envelope is versioned and carries no clock value, freshness or
/// cancellation state. With the `serde` feature it serializes as
/// `{"v":1,"key":…,"kind":…,"value_type":…,"variant":…,"origin":{…}}` and
/// deserializes without ever failing on a version or a variant it does not
/// know: the variant stays text until a replay compares it with the running
/// declaration, so an unknown variant from a newer edge is a typed
/// [`PinRejection`], not a decode error. Unknown fields inside version 1 are
/// ignored.
///
/// A pin is only a record. A keyed enqueue whose payload embeds a pin that
/// differs only in its origin is still an idempotency conflict, because the
/// whole envelope is part of the request.
#[derive(Clone, Eq, PartialEq)]
pub struct Pinned<T: FlagValue> {
    repr: Repr,
    flag: PhantomData<fn() -> T>,
}

#[derive(Clone, Eq, PartialEq)]
pub(crate) enum Repr {
    V1(V1),
    /// A pin this build cannot use, kept verbatim so that storing it again
    /// does not destroy what a newer edge wrote.
    #[cfg(feature = "serde")]
    Unusable(wire::Unusable),
}

#[derive(Clone, Eq, PartialEq)]
pub(crate) struct V1 {
    pub(crate) key: Box<str>,
    pub(crate) kind: Box<str>,
    pub(crate) value_type: Box<str>,
    pub(crate) variant: Box<str>,
    pub(crate) origin: PinOrigin,
}

impl<T: FlagValue> Pinned<T> {
    fn from_repr(repr: Repr) -> Self {
        Self {
            repr,
            flag: PhantomData,
        }
    }

    /// A pin taken from `flag`'s evaluation.
    pub(crate) fn taken(
        (key, kind, value_type): (&str, &str, &str),
        variant: &str,
        origin: PinOrigin,
    ) -> Self {
        Self::from_repr(Repr::V1(V1 {
            key: Box::from(key),
            kind: Box::from(kind),
            value_type: Box::from(value_type),
            variant: Box::from(variant),
            origin,
        }))
    }

    /// The usable body of the pin, or why it cannot be used.
    pub(crate) fn body(&self) -> Result<&V1, PinRejection> {
        match &self.repr {
            Repr::V1(body) => Ok(body),
            #[cfg(feature = "serde")]
            Repr::Unusable(unusable) => Err(unusable.rejection()),
        }
    }

    /// The pinned flag's key as stored, or `None` for a pin this build cannot
    /// read.
    pub fn key(&self) -> Option<&str> {
        self.body().ok().map(|body| &*body.key)
    }

    /// The pinned variant as stored, or `None` for a pin this build cannot
    /// read.
    pub fn variant(&self) -> Option<&str> {
        self.body().ok().map(|body| &*body.variant)
    }

    /// The pin's origin, or `None` for a pin this build cannot read.
    pub fn origin(&self) -> Option<PinOrigin> {
        self.body().ok().map(|body| body.origin)
    }

    /// Whether two pins record the same decision: the same key and variant,
    /// whatever their origin. A pin this build cannot read records no decision
    /// and equals nothing.
    pub fn same_decision(&self, other: &Self) -> bool {
        match (self.body(), other.body()) {
            (Ok(left), Ok(right)) => left.key == right.key && left.variant == right.variant,
            _ => false,
        }
    }
}

impl<T: FlagValue> fmt::Debug for Pinned<T> {
    /// Shows the stored key and variant only when they are valid names:
    /// stored text is the application's and may be anything.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.body() {
            Ok(body) => formatter
                .debug_struct("Pinned")
                .field("key", &name_or_placeholder(&body.key))
                .field("variant", &name_or_placeholder(&body.variant))
                .field("origin", &body.origin.class())
                .finish(),
            Err(rejection) => formatter
                .debug_struct("Pinned")
                .field("unusable", &rejection)
                .finish(),
        }
    }
}
