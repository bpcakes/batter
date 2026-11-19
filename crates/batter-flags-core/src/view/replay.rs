//! Replaying a stored pin against the running declaration: the checks
//! `SnapshotView::resolve_pinned` runs, in their fixed order, before an
//! incompatibility becomes a refusal.

use super::{Control, Loaded, Mode, SnapshotView};
use crate::{
    check::{HandleRefusal, Reach},
    decision::{Decision, KillSwitchKey, Layer, Reason, Source},
    pin::{PinOrigin, PinRejection, Pinnable, Pinned},
    value::FlagValue,
};

/// What replaying a pin found, before an incompatibility becomes a refusal.
pub(super) enum Replay<T> {
    Rejected(PinRejection),
    Overridden {
        decision: Decision<T>,
        by: KillSwitchKey,
    },
    Honoured(Decision<T>),
}

impl SnapshotView {
    pub(super) fn replay<F: Pinnable>(&self, flag: &F, pin: &Pinned<F::Value>) -> Replay<F::Value> {
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
        let Some((declared, pinned)) = entry
            .variants
            .iter()
            .find(|declared| ***declared == *body.variant)
            .and_then(|declared| Some((*declared, F::Value::from_variant(declared)?)))
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
        // The process that took the pin may have run in the flag's class, and
        // this one outside it evaluates the flag at its code default only.
        if self.base.outside_class[handle.index()] && entry.default_variant != Some(declared) {
            return Replay::Rejected(PinRejection::OutsideClass);
        }
        // The process that took the pin may have had settings this one lacks:
        // a row selecting the variant would be held here, so the pin is too.
        if self.base.settings_absent[handle.index()].contains(&declared) {
            return Replay::Rejected(PinRejection::RequiresSettingsAbsent);
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
}
