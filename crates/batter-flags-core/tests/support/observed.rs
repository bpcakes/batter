//! What the event tests observe: the events a capture recorded, and a view of
//! every kind over one base.
#![allow(dead_code, reason = "shared support; each test crate uses part of it")]

use batter_flags_core::{
    Freshness, PinOrigin, PrePublication, SnapshotView, Source, SwitchState, Toggle,
    low_level::Base,
};

use crate::{
    capture::{Capture, Event},
    world::{Handles, Literal, base, file_snapshot, flag_row, snapshot, switch_row, world},
};

pub const NAME: &str = "feature_flag.evaluation";
pub const TARGET: &str = "batter::flags";

pub fn evaluations(capture: &Capture) -> Vec<Event> {
    capture
        .events()
        .into_iter()
        .filter(|event| event.name == NAME)
        .collect()
}

pub fn only(capture: &Capture) -> Event {
    let mut events = evaluations(capture);
    assert_eq!(events.len(), 1, "{events:?}");
    events.remove(0)
}

/// `feature_flag.version` as the plan spells it: the source's version or
/// revision, a pin's being its origin's.
pub fn version_text(source: Source) -> String {
    let origin = |origin: PinOrigin| match origin {
        PinOrigin::Declared { revision } | PinOrigin::NotLoaded { revision } => revision.to_hex(),
        PinOrigin::ControlPlane { version }
        | PinOrigin::PersistedFile { version }
        | PinOrigin::Test { version } => version.get().to_string(),
    };
    match source {
        Source::Declared { revision } | Source::NotLoaded { revision } => revision.to_hex(),
        Source::ControlPlane { version, .. }
        | Source::PersistedFile { version }
        | Source::Test { version, .. } => version.get().to_string(),
        Source::Pinned { origin: stored } => origin(stored),
    }
}

/// A base of the serving kind and every kind of view over it, captured under
/// `capture`.
pub fn views(capture: &Capture) -> (Handles, Base, Vec<(&'static str, SnapshotView)>) {
    let (catalog, h) = world();
    let base = base(catalog, &h.serving, &[Literal::Flag("new_checkout", "on")]);
    let rows = || {
        [
            flag_row(&h.fast_path, Toggle::On, 6),
            switch_row(&h.checkout_halt, SwitchState::Engaged, 6),
        ]
    };
    let published = snapshot(&base, 7, rows());
    let restored = file_snapshot(&base, 7, rows());
    let views = capture.within(|| {
        vec![
            ("declared", base.declared_view()),
            (
                "not loaded, serving defaults",
                base.not_loaded_view(PrePublication::ServeDefaults, Freshness::Fresh),
            ),
            (
                "not loaded, unready",
                base.not_loaded_view(PrePublication::Unready, Freshness::Stale),
            ),
            (
                "control plane",
                base.control_plane_view(&published, Freshness::Fresh)
                    .unwrap(),
            ),
            (
                "stale control plane",
                base.control_plane_view(&published, Freshness::Stale)
                    .unwrap(),
            ),
            (
                "persisted file",
                base.persisted_file_view(&restored, Freshness::Fresh)
                    .unwrap(),
            ),
            (
                "test",
                base.test_view(&published, Freshness::Fresh).unwrap(),
            ),
        ]
    });
    (h, base, views)
}
