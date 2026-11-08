//! Evaluation: every row of the reason table, written out, plus governance,
//! foreign and not-owned handles and typed values.
#![allow(clippy::unwrap_used, reason = "a panic fails the test")]
#![allow(clippy::missing_assert_message, reason = "test names locate failures")]

#[path = "support/fixtures.rs"]
mod fixtures;
#[path = "support/world.rs"]
mod world;

use std::sync::Arc;

use batter_flags_core::{
    Decision, ErrorCause, Freshness, Layer, PrePublication, Reason, SnapshotView, Source,
    SwitchState, Toggle,
    low_level::{Candidate, HoldCause, Snapshot},
};
use fixtures::Checkout;
use world::{
    Literal, base, file_snapshot, flag_row, snapshot, switch_row, undecodable_row, version, world,
};

#[test]
fn views_cross_threads() {
    fn shareable<T: Send + Sync + 'static>() {}
    shareable::<SnapshotView>();
    shareable::<Decision<Toggle>>();
    shareable::<Decision<SwitchState>>();
}

/// Row: declared mode, literal present or absent.
#[test]
fn declared_mode_serves_the_literal_or_the_default() {
    let (catalog, h) = world();
    let base = base(
        catalog,
        &h.serving,
        &[
            Literal::Flag("fast_path", "on"),
            Literal::Switch("checkout_halt", SwitchState::Engaged),
        ],
    );
    let view = base.declared_view();
    let source = Source::Declared {
        revision: base.revision(),
    };
    assert_eq!(view.source(), source);
    assert_eq!(view.freshness(), Freshness::Fresh);

    let literal = view.evaluate(&h.fast_path);
    assert_eq!(literal.value(), Toggle::On);
    assert_eq!(literal.variant(), "on");
    assert_eq!(literal.reason(), Reason::Static);
    assert_eq!(literal.layer(), &Layer::Entry);
    assert_eq!(literal.source(), source);
    assert_eq!(literal.error(), None);

    let absent = view.evaluate(&h.checkout_style);
    assert_eq!(absent.value(), Checkout::Classic);
    assert_eq!(absent.variant(), "classic");
    assert_eq!(absent.reason(), Reason::Default);
    assert_eq!(absent.layer(), &Layer::Base);

    let engaged = view.switch(&h.checkout_halt);
    assert_eq!(engaged.value(), SwitchState::Engaged);
    assert_eq!(engaged.variant(), "engaged");
    assert_eq!(engaged.reason(), Reason::Static);
    assert_eq!(engaged.layer(), &Layer::Entry);
}

#[test]
fn declared_mode_serves_a_switch_its_safe_state_when_no_literal_is_present() {
    let (catalog, h) = world();
    let base = base(catalog, &h.worker, &[]);
    let view = base.declared_view();
    let engaged = view.switch(&h.export_halt);
    assert_eq!(
        (engaged.value(), engaged.reason(), engaged.layer()),
        (SwitchState::Engaged, Reason::Default, &Layer::Base)
    );
    let released = view.switch(&h.checkout_halt);
    assert_eq!(
        (released.value(), released.reason(), released.layer()),
        (SwitchState::Released, Reason::Default, &Layer::Base)
    );
}

/// Row: a control plane is configured and nothing is published yet.
#[test]
fn before_the_first_publication_the_load_policy_decides() {
    let (catalog, h) = world();
    let base = base(catalog, &h.serving, &[Literal::Flag("fast_path", "on")]);

    let serve = base.not_loaded_view(PrePublication::ServeDefaults, Freshness::Fresh);
    let source = Source::NotLoaded {
        revision: base.revision(),
    };
    assert_eq!(serve.source(), source);
    // The base is served as the base, even where a setting supplied it.
    let served = serve.evaluate(&h.fast_path);
    assert_eq!(
        (
            served.value(),
            served.reason(),
            served.layer(),
            served.source()
        ),
        (Toggle::On, Reason::Default, &Layer::Base, source)
    );
    let unset = serve.evaluate(&h.checkout_style);
    assert_eq!(
        (unset.value(), unset.reason(), unset.layer()),
        (Checkout::Classic, Reason::Default, &Layer::Base)
    );
    let switch = serve.switch(&h.checkout_halt);
    assert_eq!(
        (switch.value(), switch.reason(), switch.layer()),
        (SwitchState::Released, Reason::Default, &Layer::Base)
    );

    let unready = base.not_loaded_view(PrePublication::Unready, Freshness::Fresh);
    let flag = unready.evaluate(&h.fast_path);
    assert_eq!(flag.reason(), Reason::Error);
    assert_eq!(flag.error(), Some(ErrorCause::ProviderNotReady));
    assert_eq!(flag.value(), Toggle::On, "the base value is still reported");
    assert_eq!(flag.layer(), &Layer::Base);
    let switch = unready.switch(&h.checkout_halt);
    assert_eq!(switch.reason(), Reason::Error);
    assert_eq!(switch.error(), Some(ErrorCause::ProviderNotReady));
    assert_eq!(switch.value(), SwitchState::Released);
    assert_eq!(base.error_count(ErrorCause::ProviderNotReady), 2);
}

/// Row: a published entry, and when it reads as stale.
#[test]
fn a_published_entry_is_static_unless_stale_or_from_the_file() {
    let (catalog, h) = world();
    let base = base(catalog, &h.serving, &[]);
    let rows = || {
        [
            flag_row(&h.fast_path, Toggle::On, 6),
            switch_row(&h.checkout_halt, SwitchState::Engaged, 5),
        ]
    };
    let snapshot = snapshot(&base, 7, rows());
    let restored = file_snapshot(&base, 7, rows());
    let cases = [
        ("control plane, fresh", Freshness::Fresh, Reason::Static),
        ("control plane, stale", Freshness::Stale, Reason::Stale),
    ];
    for (label, freshness, reason) in cases {
        let view = base.control_plane_view(&snapshot, freshness).unwrap();
        assert_eq!(
            view.source(),
            Source::ControlPlane {
                version: version(7),
                freshness
            },
            "{label}"
        );
        let flag = view.evaluate(&h.fast_path);
        assert_eq!(
            (flag.value(), flag.reason(), flag.layer()),
            (Toggle::On, reason, &Layer::Entry),
            "{label}"
        );
        assert_eq!(flag.source(), view.source(), "{label}");
        let switch = view.switch(&h.checkout_halt);
        // checkout_halt holds its last state, so a stale view still serves the entry.
        assert_eq!(
            (switch.value(), switch.reason(), switch.layer()),
            (SwitchState::Engaged, reason, &Layer::Entry),
            "{label}"
        );
    }
    for freshness in [Freshness::Fresh, Freshness::Stale] {
        let view = base.persisted_file_view(&restored, freshness).unwrap();
        assert_eq!(
            view.source(),
            Source::PersistedFile {
                version: version(7)
            }
        );
        let flag = view.evaluate(&h.fast_path);
        assert_eq!(
            (flag.value(), flag.reason(), flag.layer()),
            (Toggle::On, Reason::Stale, &Layer::Entry),
            "a file's entries are reported stale however fresh the view"
        );
    }
    for (freshness, reason) in [
        (Freshness::Fresh, Reason::Static),
        (Freshness::Stale, Reason::Stale),
    ] {
        let view = base.test_view(&snapshot, freshness).unwrap();
        assert_eq!(
            view.source(),
            Source::Test {
                version: version(7),
                freshness
            }
        );
        let flag = view.evaluate(&h.fast_path);
        assert_eq!((flag.value(), flag.reason()), (Toggle::On, reason));
    }
}

/// Row: no entry (a store loads an archived key as absent), held, or a row for
/// another kind.
#[test]
fn without_a_usable_entry_the_base_is_served() {
    let (catalog, h) = world();
    let base = base(
        catalog,
        &h.serving,
        &[Literal::Flag("checkout_style", "express")],
    );
    let snapshot = snapshot(
        &base,
        4,
        [
            // `fast_path` has no row at all
            // held: an encoding this build cannot read
            undecodable_row("checkout_style", "mode", "enum", 4),
            // held: a well-formed row for a declaration the control plane never supplies
            flag_row(&h.debug_panel, Toggle::On, 2),
            // a row for a key no declaration has, and one for a kind that does not own it
            undecodable_row("retired_flag", "mode", "toggle", 1),
            flag_row(&h.worker_mode, Toggle::On, 2),
            // a held kill switch with nothing last published
            undecodable_row("checkout_halt", "switch", "switch", 4),
        ],
    );
    assert_eq!((snapshot.unknown(), snapshot.not_owned()), (1, 1));
    let held: Vec<_> = snapshot
        .held(batter_flags_core::Freshness::Fresh)
        .map(|(key, cause)| (key.as_str().to_owned(), cause))
        .collect();
    assert_eq!(
        held,
        [
            ("checkout_halt".to_owned(), HoldCause::UnknownRowEncoding),
            ("checkout_style".to_owned(), HoldCause::UnknownRowEncoding),
            ("debug_panel".to_owned(), HoldCause::EnvironmentRestricted),
        ],
        "declaration order, one cause each"
    );
    let view = base
        .control_plane_view(&snapshot, Freshness::Fresh)
        .unwrap();

    let absent = view.evaluate(&h.fast_path);
    assert_eq!(
        (absent.value(), absent.reason(), absent.layer()),
        (Toggle::Off, Reason::Default, &Layer::Base)
    );
    let held = view.evaluate(&h.checkout_style);
    assert_eq!(
        (held.value(), held.reason(), held.layer()),
        (Checkout::Express, Reason::Default, &Layer::Base),
        "a held flag serves the declared base, including a setting's literal"
    );
    assert_eq!(held.error(), None, "holding never reports an error");
    let restricted = view.evaluate(&h.debug_panel);
    assert_eq!(
        (restricted.value(), restricted.reason(), restricted.layer()),
        (Toggle::Off, Reason::Default, &Layer::Base),
        "the published `on` of a restricted flag is never served"
    );
    let switch = view.switch(&h.checkout_halt);
    assert_eq!(
        (switch.value(), switch.reason(), switch.layer()),
        (SwitchState::Released, Reason::Default, &Layer::Base),
        "a held switch with nothing last published serves its safe state"
    );
    for cause in ErrorCause::ALL {
        assert_eq!(
            base.error_count(cause),
            0,
            "{cause:?}: holding is not an error"
        );
    }
}

/// Row: a held kill switch with a last published state.
#[test]
fn a_held_switch_keeps_the_state_it_last_published() {
    let (catalog, h) = world();
    let base = base(catalog, &h.serving, &[]);
    // The row was written at 3 and served by the snapshot read at 8.
    let first = snapshot(
        &base,
        8,
        [switch_row(&h.checkout_halt, SwitchState::Engaged, 3)],
    );
    let scope = base.validation_scope();
    let held = Candidate::new(&scope)
        .at(version(9))
        .row(undecodable_row("checkout_halt", "switch", "switch", 9))
        .carrying(&first)
        .unwrap();
    let held = Arc::new(Snapshot::validate(&scope, held).unwrap());
    let view = base.control_plane_view(&held, Freshness::Fresh).unwrap();
    let switch = view.switch(&h.checkout_halt);
    assert_eq!(
        switch.value(),
        SwitchState::Engaged,
        "holding never releases"
    );
    assert_eq!(switch.layer(), &Layer::Held { since: version(8) });
    assert_eq!(switch.reason(), Reason::Stale);
    assert_eq!(
        switch.source(),
        Source::ControlPlane {
            version: version(9),
            freshness: Freshness::Fresh
        },
        "the source is the view's snapshot whichever layer supplied the value"
    );
    // The carried state survives a second held read, keeping its original `since`.
    let again = Candidate::new(&scope)
        .at(version(12))
        .row(undecodable_row("checkout_halt", "switch", "switch", 12))
        .carrying(&held)
        .unwrap();
    let again = Arc::new(Snapshot::validate(&scope, again).unwrap());
    let view = base.control_plane_view(&again, Freshness::Fresh).unwrap();
    assert_eq!(
        view.switch(&h.checkout_halt).layer(),
        &Layer::Held { since: version(8) }
    );
}

/// Row: a switch engaged by its settings floor.
#[test]
fn an_engaged_literal_is_a_floor_a_released_literal_is_nothing() {
    let (catalog, h) = world();
    let floored = base(
        catalog,
        &h.serving,
        &[Literal::Switch("checkout_halt", SwitchState::Engaged)],
    );
    let released_row = snapshot(
        &floored,
        2,
        [switch_row(&h.checkout_halt, SwitchState::Released, 2)],
    );
    let before = floored.not_loaded_view(PrePublication::Unready, Freshness::Fresh);
    let after = floored
        .control_plane_view(&released_row, Freshness::Fresh)
        .unwrap();
    for (label, view) in [
        ("before publication", &before),
        ("after publication", &after),
    ] {
        let switch = view.switch(&h.checkout_halt);
        assert_eq!(
            (
                switch.value(),
                switch.reason(),
                switch.layer(),
                switch.error()
            ),
            (SwitchState::Engaged, Reason::Static, &Layer::Floor, None),
            "{label}: no row releases a floor"
        );
    }

    let (catalog, h) = world();
    let ignored = base(
        catalog,
        &h.serving,
        &[Literal::Switch("checkout_halt", SwitchState::Released)],
    );
    let engaged_row = snapshot(
        &ignored,
        2,
        [switch_row(&h.checkout_halt, SwitchState::Engaged, 2)],
    );
    let view = ignored
        .control_plane_view(&engaged_row, Freshness::Fresh)
        .unwrap();
    let switch = view.switch(&h.checkout_halt);
    assert_eq!(
        (switch.value(), switch.layer()),
        (SwitchState::Engaged, &Layer::Entry),
        "a released literal neither floors nor overrides the control plane"
    );
}

/// Row: a stale view engaging an `EngageWhenStale` switch.
#[test]
fn a_stale_view_applies_each_switchs_stale_rule() {
    let (catalog, h) = world();
    let base = base(catalog, &h.worker, &[]);
    let snapshot = snapshot(
        &base,
        8,
        [
            switch_row(&h.export_halt, SwitchState::Released, 8),
            switch_row(&h.checkout_halt, SwitchState::Released, 8),
        ],
    );
    let fresh = base
        .control_plane_view(&snapshot, Freshness::Fresh)
        .unwrap();
    assert_eq!(fresh.switch(&h.export_halt).value(), SwitchState::Released);

    let stale = base
        .control_plane_view(&snapshot, Freshness::Stale)
        .unwrap();
    let engaging = stale.switch(&h.export_halt);
    assert_eq!(
        (engaging.value(), engaging.reason(), engaging.layer()),
        (SwitchState::Engaged, Reason::Stale, &Layer::StaleRule)
    );
    let holding = stale.switch(&h.checkout_halt);
    assert_eq!(
        (holding.value(), holding.reason(), holding.layer()),
        (SwitchState::Released, Reason::Stale, &Layer::Entry),
        "HoldLastKnown keeps its served state"
    );
    // A boot-time outage: nothing published and already stale.
    let boot = base.not_loaded_view(PrePublication::ServeDefaults, Freshness::Stale);
    assert_eq!(boot.switch(&h.export_halt).layer(), &Layer::StaleRule);
    assert_eq!(
        (
            boot.switch(&h.checkout_halt).value(),
            boot.switch(&h.checkout_halt).layer()
        ),
        (SwitchState::Released, &Layer::Base)
    );
}
