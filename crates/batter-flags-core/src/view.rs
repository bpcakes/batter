//! The snapshot view: one frozen evaluation context.
//!
//! A view captures a [`Base`], at most one [`Snapshot`], a source, the
//! freshness its publisher computed and the dispatcher and span current at
//! capture. Evaluation reads only that captured state, is synchronous,
//! infallible and does no I/O.
//!
//! # Layers
//!
//! A release or mode flag layers a published entry over the declared base.
//! The base is the setting's literal if the process had one, else the code
//! default; declared mode serves the literal as an entry instead.
//!
//! A kill switch is engaged if, in this order:
//!
//! 1. its settings floor is set (an explicit `engaged` literal), whatever is
//!    published and before anything is;
//! 2. the view is stale and the switch's rule is `EngageWhenStale`;
//! 3. its served state is engaged: the published entry, or for a held entry
//!    the state it last published, else its safe state.
//!
//! Declared mode has no control plane: the literal if present, else the safe
//! state. A release flag whose governing switch is engaged serves its code
//! default with [`Layer::Override`], live or pinned.

mod decide;
mod gate;

use std::{fmt, sync::Arc};

pub use self::gate::{GateReading, gate_switch};
use crate::{
    base::{Base, BaseInner},
    check::{HandleRefusal, Reach},
    decision::{
        Decision, ErrorCause, Freshness, KillSwitchKey, Layer, PrePublication, Reason, Source,
        SwitchState,
    },
    events::{self, Evaluation, EventContext, Surface},
    handle::{Flag, Handle, KillSwitch},
    pin::{PinOrigin, PinRefusal, PinRejection, Pinnable, Pinned, PinnedResolution, Unpinnable},
    snapshot::{Provenance, Snapshot},
    value::FlagValue,
};

/// Why a view could not be built from a snapshot.
#[derive(Clone, Copy, Debug, Eq, thiserror::Error, PartialEq)]
pub enum ViewError {
    /// The snapshot was validated for another base.
    #[error("the snapshot was validated for another base")]
    ForeignSnapshot,
    /// The snapshot was read from the other source: a view names the source of
    /// its snapshot, so a file snapshot cannot back a control-plane view or the
    /// reverse.
    #[error("a {view} view cannot be built from a snapshot read from the {}", .found.as_str())]
    WrongProvenance {
        /// The view's source: `control_plane` or `persisted_file`.
        view: &'static str,
        /// Where the snapshot was read from.
        found: Provenance,
    },
}

/// What replaying a pin found, before an incompatibility becomes a refusal.
enum Replay<T> {
    Rejected(PinRejection),
    Overridden {
        decision: Decision<T>,
        by: KillSwitchKey,
    },
    Honoured(Decision<T>),
}

/// Which loaded source a view reports.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Loaded {
    ControlPlane,
    PersistedFile,
    Test,
}

/// What a control-plane view has published.
enum Control {
    /// Nothing is published yet.
    NotLoaded(PrePublication),
    Loaded {
        snapshot: Arc<Snapshot>,
        kind: Loaded,
    },
}

enum Mode {
    Declared,
    Control(Control),
}

/// One frozen evaluation context: what a request, a job invocation or a
/// periodic run evaluates against.
///
/// A view has private fields and no public constructor: it is built from a
/// [`Base`] through the disclosed [`low_level`](crate::low_level) path, by a
/// publisher that asserts its [`Source`] and [`Freshness`]. A view never
/// changes after capture; evaluation reflects the state it captured however
/// long it lives.
///
/// ```compile_fail,E0451
/// use batter_flags_core::SnapshotView;
///
/// fn forge() -> SnapshotView {
///     SnapshotView {
///         base: todo!(),
///         mode: todo!(),
///         freshness: todo!(),
///         context: todo!(),
///     }
/// }
/// ```
///
/// ```
/// use batter_flags_core::{
///     CatalogBuilder, CivilDate, Expiry, Flag, Layer, Reason, SettingsPrefix, Toggle,
///     low_level::BaseBuilder,
/// };
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let (mut catalog, serving, _) = CatalogBuilder::new(SettingsPrefix::new("FLAG_")?, "serving", "hosted")?;
/// let expiry = Expiry::on(CivilDate::new(2027, 3, 31)?);
/// let new_checkout = catalog.release_flag("new_checkout", Toggle::Off, expiry).declare()?;
///
/// let mut base = BaseBuilder::new(catalog.build(), serving)?;
/// base.flag(new_checkout.key(), "on")?;
/// let base = base.finish();
///
/// // Declared mode: the setting's literal is an entry.
/// let decision = base.declared_view().evaluate(&new_checkout);
/// assert_eq!(decision.value(), Toggle::On);
/// assert_eq!(decision.reason(), Reason::Static);
/// assert_eq!(decision.layer(), &Layer::Entry);
/// # Ok(())
/// # }
/// ```
pub struct SnapshotView {
    base: Arc<BaseInner>,
    mode: Mode,
    freshness: Freshness,
    context: EventContext,
}

impl SnapshotView {
    fn new(base: &Base, mode: Mode, freshness: Freshness) -> Self {
        Self {
            base: Arc::clone(&base.inner),
            mode,
            freshness,
            context: EventContext::capture(),
        }
    }

    /// Where this view's decisions get their information.
    pub fn source(&self) -> Source {
        match &self.mode {
            Mode::Declared => Source::Declared {
                revision: self.base.revision,
            },
            Mode::Control(Control::NotLoaded(_)) => Source::NotLoaded {
                revision: self.base.revision,
            },
            Mode::Control(Control::Loaded { snapshot, kind }) => match kind {
                Loaded::ControlPlane => Source::ControlPlane {
                    version: snapshot.version,
                    freshness: self.freshness,
                },
                Loaded::PersistedFile => Source::PersistedFile {
                    version: snapshot.version,
                },
                Loaded::Test => Source::Test {
                    version: snapshot.version,
                    freshness: self.freshness,
                },
            },
        }
    }

    /// The freshness the publisher asserted at capture. Declared mode has no
    /// control plane and is always fresh.
    pub fn freshness(&self) -> Freshness {
        self.freshness
    }

    /// Evaluates a release or mode flag, and emits one evaluation event.
    ///
    /// A handle from another catalog, or for a flag this process kind does not
    /// own, yields its code default with reason `error`.
    pub fn evaluate<F: Flag>(&self, flag: &F) -> Decision<F::Value> {
        self.evaluate_handle(flag.handle(), flag.code_default())
    }

    /// Evaluates the flag a handle names and emits one evaluation event.
    pub(crate) fn evaluate_handle<T: FlagValue>(&self, handle: &Handle, default: T) -> Decision<T> {
        let decision = self.flag_decision(handle, default);
        self.report(handle, &decision, Surface::Ordinary);
        decision
    }

    /// Reads a kill switch, and emits one evaluation event.
    pub fn switch(&self, switch: &KillSwitch) -> Decision<SwitchState> {
        let decision = self.switch_decision(switch);
        self.report(switch.handle(), &decision, Surface::Ordinary);
        decision
    }

    /// Pins the decision this view serves for `flag`: its variant and this
    /// view's origin, ready to store in the application's own row.
    ///
    /// The pin records the effective decision, so a flag whose governing
    /// switch is engaged is pinned at its code default. A replay still checks
    /// the governing switch of the view it runs under. Emits one evaluation
    /// event.
    ///
    /// # Errors
    ///
    /// [`Unpinnable`] when the decision is an error: a handle this view cannot
    /// answer for, such as another catalog's, or a view captured before the
    /// first publication under a load policy that withholds readiness, even
    /// when an engaged governing switch would answer `evaluate`. The error is
    /// counted and reported as [`evaluate`](Self::evaluate) counts one.
    pub fn pin<F: Pinnable>(&self, flag: &F) -> Result<Pinned<F::Value>, Unpinnable> {
        let handle = flag.handle();
        let default = flag.code_default();
        let decision = match &self.mode {
            // Nothing is published and readiness is withheld: a governed
            // decision is not one to replay as cached once the switch is
            // released, so the view's not-ready error is what is reported.
            Mode::Control(Control::NotLoaded(PrePublication::Unready)) => Decision::failed(
                (default, default.variant()),
                ErrorCause::ProviderNotReady,
                self.source(),
                Layer::Base,
            ),
            Mode::Declared
            | Mode::Control(Control::NotLoaded(PrePublication::ServeDefaults))
            | Mode::Control(Control::Loaded { .. }) => self.flag_decision(handle, default),
        };
        self.report(handle, &decision, Surface::Ordinary);
        if let Some(cause) = decision.error() {
            return Err(Unpinnable::new(cause));
        }
        Ok(Pinned::taken(
            (
                flag.key().as_str(),
                handle.kind().as_str(),
                handle.value_type().as_str(),
            ),
            decision.variant(),
            PinOrigin::of(self.source()),
        ))
    }

    /// Replays a stored pin and emits one evaluation event.
    ///
    /// The handle is checked first, then the pin against the running
    /// declaration, then the governing kill switch, so a mismatched pin is
    /// never reported as [`PinnedResolution::Overridden`]. A kind may replay
    /// pins of a flag it owns or whose `pinned_by` lists it. An incompatible
    /// pin's event carries no variant, and its [`PinRefusal`] is the only way
    /// to read the flag live in its place.
    pub fn resolve_pinned<'v, F: Pinnable>(
        &'v self,
        flag: &F,
        pin: &Pinned<F::Value>,
    ) -> PinnedResolution<'v, F::Value> {
        let handle = flag.handle();
        match self.replay(flag, pin) {
            Replay::Rejected(rejection) => {
                self.report_rejection(handle, rejection);
                PinnedResolution::Incompatible(PinRefusal::new(
                    self,
                    handle.clone(),
                    flag.code_default(),
                    rejection,
                ))
            }
            Replay::Overridden { decision, by } => {
                self.report(handle, &decision, Surface::Ordinary);
                PinnedResolution::Overridden { decision, by }
            }
            Replay::Honoured(decision) => {
                self.report(handle, &decision, Surface::Ordinary);
                PinnedResolution::Honoured(decision)
            }
        }
    }

    fn replay<F: Pinnable>(&self, flag: &F, pin: &Pinned<F::Value>) -> Replay<F::Value> {
        let handle = flag.handle();
        if let Some(refusal) = self.refusal(handle, Reach::OwnerOrPinner) {
            return Replay::Rejected(match refusal {
                HandleRefusal::ForeignCatalog => PinRejection::ForeignHandle,
                HandleRefusal::NotOwned => PinRejection::NotOwned,
            });
        }
        let body = match pin.body() {
            Ok(body) => body,
            Err(rejection) => return Replay::Rejected(rejection),
        };
        let entry = self.catalog_entry(handle);
        let rejection = if *body.key != *flag.key().as_str() {
            Some(PinRejection::KeyMismatch)
        } else if *body.kind != *handle.kind().as_str() {
            Some(PinRejection::KindChanged)
        } else if *body.value_type != *handle.value_type().as_str() {
            Some(PinRejection::ValueTypeChanged)
        } else {
            None
        };
        if let Some(rejection) = rejection {
            return Replay::Rejected(rejection);
        }
        let Some(pinned) = entry
            .variants
            .iter()
            .find(|declared| ***declared == *body.variant)
            .and_then(|declared| F::Value::from_variant(declared))
        else {
            return Replay::Rejected(PinRejection::UnknownVariant);
        };
        let is_test_view = matches!(
            self.mode,
            Mode::Control(Control::Loaded {
                kind: Loaded::Test,
                ..
            })
        );
        if matches!(body.origin, PinOrigin::Test { .. }) && !is_test_view {
            return Replay::Rejected(PinRejection::TestProvenance);
        }
        if let Some(governing) = &entry.governed_by
            && self.switch_outcome(governing.index).state.is_engaged()
        {
            return Replay::Overridden {
                decision: self.overridden(flag.code_default(), &governing.key),
                by: KillSwitchKey::new(governing.key.clone()),
            };
        }
        Replay::Honoured(Decision::served(
            (pinned, pinned.variant()),
            Reason::Cached,
            Source::Pinned {
                origin: body.origin,
            },
            Layer::Entry,
        ))
    }

    fn report<T>(&self, handle: &Handle, decision: &Decision<T>, surface: Surface) {
        events::emit(
            &self.context,
            &self.base.tally,
            &self.base.catalog.digest,
            &Evaluation {
                key: handle.key().as_str(),
                variant: Some(decision.variant()),
                reason: decision.reason(),
                version: decision.source().version_text(),
                error: decision
                    .error()
                    .map(|cause| (cause, (handle.catalog(), handle.index()))),
                surface,
            },
        );
    }

    fn report_rejection(&self, handle: &Handle, rejection: PinRejection) {
        events::emit(
            &self.context,
            &self.base.tally,
            &self.base.catalog.digest,
            &Evaluation {
                key: handle.key().as_str(),
                variant: None,
                reason: Reason::Error,
                version: self.source().version_text(),
                error: Some((rejection.cause(), (handle.catalog(), handle.index()))),
                surface: Surface::Ordinary,
            },
        );
    }
}

impl fmt::Debug for SnapshotView {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SnapshotView")
            .field("source", &self.source())
            .field("freshness", &self.freshness)
            .finish_non_exhaustive()
    }
}

impl Base {
    /// A view of declared mode: the settings read at startup, as
    /// [`Source::Declared`].
    pub fn declared_view(&self) -> SnapshotView {
        SnapshotView::new(self, Mode::Declared, Freshness::Fresh)
    }

    /// A view captured before anything was published, as
    /// [`Source::NotLoaded`].
    ///
    /// `pre_publication` is the load policy's class, and `freshness` is
    /// whatever the publisher asserts: a process that has waited longer than
    /// the maximum age is stale and applies each switch's stale rule.
    pub fn not_loaded_view(
        &self,
        pre_publication: PrePublication,
        freshness: Freshness,
    ) -> SnapshotView {
        SnapshotView::new(
            self,
            Mode::Control(Control::NotLoaded(pre_publication)),
            freshness,
        )
    }

    /// A view of a snapshot loaded from the control plane, as
    /// [`Source::ControlPlane`].
    ///
    /// # Errors
    ///
    /// [`ViewError::ForeignSnapshot`] when `snapshot` was validated for another
    /// base, and [`ViewError::WrongProvenance`] when it was read from the
    /// last-good file.
    pub fn control_plane_view(
        &self,
        snapshot: &Arc<Snapshot>,
        freshness: Freshness,
    ) -> Result<SnapshotView, ViewError> {
        self.loaded_view(snapshot, Loaded::ControlPlane, freshness)
    }

    /// A view of a snapshot restored from the last-good file, as
    /// [`Source::PersistedFile`]. Its entries report reason `stale`: the file
    /// carries no clock.
    ///
    /// # Errors
    ///
    /// [`ViewError::ForeignSnapshot`] when `snapshot` was validated for another
    /// base, and [`ViewError::WrongProvenance`] when it was read from the
    /// control plane.
    pub fn persisted_file_view(
        &self,
        snapshot: &Arc<Snapshot>,
        freshness: Freshness,
    ) -> Result<SnapshotView, ViewError> {
        self.loaded_view(snapshot, Loaded::PersistedFile, freshness)
    }

    /// A view of injected test flags, as [`Source::Test`]: it derives reasons
    /// like a control-plane view, and a test pin replays only under it. It
    /// accepts a snapshot of either provenance, because it claims neither.
    ///
    /// # Errors
    ///
    /// [`ViewError::ForeignSnapshot`] when `snapshot` was validated for another
    /// base.
    pub fn test_view(
        &self,
        snapshot: &Arc<Snapshot>,
        freshness: Freshness,
    ) -> Result<SnapshotView, ViewError> {
        self.loaded_view(snapshot, Loaded::Test, freshness)
    }

    fn loaded_view(
        &self,
        snapshot: &Arc<Snapshot>,
        kind: Loaded,
        freshness: Freshness,
    ) -> Result<SnapshotView, ViewError> {
        if !Arc::ptr_eq(&self.inner, &snapshot.base) {
            return Err(ViewError::ForeignSnapshot);
        }
        let required = match kind {
            Loaded::ControlPlane => Some(("control_plane", Provenance::ControlPlane)),
            Loaded::PersistedFile => Some(("persisted_file", Provenance::LastGoodFile)),
            Loaded::Test => None,
        };
        if let Some((view, required)) = required
            && snapshot.provenance() != required
        {
            return Err(ViewError::WrongProvenance {
                view,
                found: snapshot.provenance(),
            });
        }
        Ok(SnapshotView::new(
            self,
            Mode::Control(Control::Loaded {
                snapshot: Arc::clone(snapshot),
                kind,
            }),
            freshness,
        ))
    }
}
