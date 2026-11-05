//! The standing of what a process serves: which snapshot may be applied,
//! confirmed or refused, and where a held kill-switch state came from.
#![allow(clippy::unwrap_used, reason = "a panic fails the test")]
#![allow(clippy::missing_assert_message, reason = "test names locate failures")]

#[path = "support/fixtures.rs"]
mod fixtures;
#[path = "support/world.rs"]
mod world;

use std::sync::Arc;

use batter_flags_core::{
    Freshness, Layer, SwitchState, Toggle,
    low_level::{
        Authority, Candidate, CandidateRow, Provenance, Refusal, Snapshot, Standing, Transition,
        ViewError,
    },
};
use world::{base, file_snapshot, flag_row, snapshot, switch_row, undecodable_row, version, world};

fn applied(transition: Transition) -> Authority {
    match transition {
        Transition::Apply(authority) => authority,
        other => panic!("expected Apply, got {other:?}"),
    }
}

fn control_plane(authority: &Authority) -> (u64, batter_flags_core::SnapshotDigest) {
    match authority.standing() {
        Standing::ControlPlane { version, digest } => (version.get(), digest),
        other => panic!("expected a control-plane standing, got {other:?}"),
    }
}

/// The sequence the audit named: file version 100, then the first control-plane
/// version 5, then a regression to 4, a confirmation at 5 and drift at 5.
#[test]
fn the_standing_follows_the_file_then_the_control_plane_and_never_goes_back() {
    let (catalog, h) = world();
    let base = base(catalog, &h.serving, &[]);
    let rows = |on: Toggle| [flag_row(&h.fast_path, on, 1)];

    let unpublished = Authority::unpublished();
    assert_eq!(unpublished.standing(), Standing::Unpublished);

    // The file is the first publication and confirms nothing.
    let from_file = unpublished.consider(&file_snapshot(&base, 100, rows(Toggle::On)));
    assert!(!from_file.renews_freshness());
    let persisted = applied(from_file);
    assert_eq!(
        persisted.standing(),
        Standing::Persisted {
            version: version(100)
        }
    );

    // The first control-plane read is applied even far below the file's version.
    let first = snapshot(&base, 5, rows(Toggle::On));
    let from_control_plane = persisted.consider(&first);
    assert!(from_control_plane.renews_freshness());
    let serving = applied(from_control_plane);
    assert_eq!(control_plane(&serving), (5, first.digest()));

    // A lower version is refused and renews nothing.
    let regressed = serving.consider(&snapshot(&base, 4, rows(Toggle::On)));
    assert_eq!(
        regressed,
        Transition::Refuse(Refusal::Regressed {
            applied: version(5),
            observed: version(4)
        })
    );
    assert!(!regressed.renews_freshness());

    // The same version with the same digest confirms.
    let confirmed = serving.consider(&snapshot(&base, 5, rows(Toggle::On)));
    assert_eq!(confirmed, Transition::Confirm);
    assert!(confirmed.renews_freshness());

    // The same version with another digest is drift: nothing renews.
    let drifted = serving.consider(&snapshot(&base, 5, rows(Toggle::Off)));
    assert_eq!(
        drifted,
        Transition::Refuse(Refusal::DigestDrift {
            version: version(5)
        })
    );
    assert!(!drifted.renews_freshness());

    // A higher version is applied, with its own digest.
    let next = snapshot(&base, 6, rows(Toggle::Off));
    let later = applied(serving.consider(&next));
    assert_eq!(control_plane(&later), (6, next.digest()));
    // Refusals changed nothing: the authority that judged them is as it was.
    assert_eq!(control_plane(&serving), (5, first.digest()));
}

#[test]
fn a_file_is_only_ever_the_first_publication() {
    let (catalog, h) = world();
    let base = base(catalog, &h.serving, &[]);
    let restored = file_snapshot(&base, 100, []);
    let persisted = applied(Authority::unpublished().consider(&restored));
    let serving = applied(persisted.consider(&snapshot(&base, 5, [])));
    for authority in [persisted, serving] {
        let late = authority.consider(&file_snapshot(&base, 200, []));
        assert_eq!(late, Transition::Refuse(Refusal::FileAfterPublication));
        assert!(!late.renews_freshness());
    }
}

#[test]
fn the_digest_and_not_the_version_alone_decides_confirmation() {
    let (catalog, h) = world();
    let base = base(catalog, &h.serving, &[]);
    let read = |style: &str, updated: u64| {
        snapshot(
            &base,
            9,
            [
                flag_row(&h.fast_path, Toggle::On, updated),
                CandidateRow::new("checkout_style", "mode", "enum", version(2), style),
            ],
        )
    };
    let serving = applied(Authority::unpublished().consider(&read("held", 8)));
    assert_eq!(serving.consider(&read("held", 8)), Transition::Confirm);
    // A held row's text, and a row's own version, are part of what was read.
    for (style, updated) in [("changed", 8), ("held", 7)] {
        assert_eq!(
            serving.consider(&read(style, updated)),
            Transition::Refuse(Refusal::DigestDrift {
                version: version(9)
            }),
            "{style} {updated}"
        );
    }
}

#[test]
fn a_candidate_says_where_it_was_read_from() {
    let (catalog, h) = world();
    let base = base(catalog, &h.serving, &[]);
    assert_eq!(
        snapshot(&base, 3, []).provenance(),
        Provenance::ControlPlane
    );
    assert_eq!(
        file_snapshot(&base, 3, []).provenance(),
        Provenance::LastGoodFile
    );
    assert_eq!(Provenance::ControlPlane.as_str(), "control_plane");
    assert_eq!(Provenance::LastGoodFile.as_str(), "last_good_file");
}

/// A held switch keeps serving what it last published, and keeps saying where
/// that state was first read from, however many holds later.
#[test]
fn a_held_switch_keeps_the_provenance_of_where_its_state_was_first_read() {
    let (catalog, h) = world();
    let base = base(catalog, &h.serving, &[]);
    let scope = base.validation_scope();
    let hold = |previous: &Snapshot, at: u64| {
        let candidate = Candidate::new(&scope)
            .at(version(at))
            .rows([
                flag_row(&h.fast_path, Toggle::On, at - 1),
                undecodable_row("checkout_halt", "switch", "switch", at - 1),
            ])
            .carrying(previous)
            .unwrap();
        Arc::new(Snapshot::validate(&scope, candidate).unwrap())
    };
    let carried = |snapshot: &Snapshot| -> Vec<(String, SwitchState, u64, Provenance)> {
        snapshot
            .carried()
            .map(|held| {
                (
                    held.key().as_str().to_owned(),
                    held.state(),
                    held.since().get(),
                    held.provenance(),
                )
            })
            .collect()
    };

    // Engaged in the file, then held by the control plane, then held again.
    let restored = file_snapshot(
        &base,
        100,
        [switch_row(&h.checkout_halt, SwitchState::Engaged, 99)],
    );
    assert_eq!(
        carried(&restored),
        [],
        "a published state is not a carried one"
    );
    let first_hold = hold(&restored, 5);
    let expected = (
        "checkout_halt".to_owned(),
        SwitchState::Engaged,
        100,
        Provenance::LastGoodFile,
    );
    assert_eq!(carried(&first_hold), std::slice::from_ref(&expected));
    let second_hold = hold(&first_hold, 6);
    assert_eq!(
        carried(&second_hold),
        [expected],
        "still the file's, since version 100"
    );

    // Engaged by the control plane, then held.
    let published = snapshot(
        &base,
        7,
        [switch_row(&h.checkout_halt, SwitchState::Engaged, 6)],
    );
    assert_eq!(
        carried(&hold(&published, 8)),
        [(
            "checkout_halt".to_owned(),
            SwitchState::Engaged,
            7,
            Provenance::ControlPlane
        )]
    );
    // A hold with nothing last published carries nothing.
    assert_eq!(
        carried(&snapshot(
            &base,
            9,
            [undecodable_row("checkout_halt", "switch", "switch", 8)]
        )),
        []
    );
}

/// A view names the source of its snapshot, so the snapshot's provenance has to
/// agree; a test view claims neither source and takes either.
#[test]
fn a_view_refuses_a_snapshot_read_from_the_other_source() {
    let (catalog, h) = world();
    let base = base(catalog, &h.serving, &[]);
    let (published, restored) = (snapshot(&base, 3, []), file_snapshot(&base, 3, []));

    assert_eq!(
        base.control_plane_view(&restored, Freshness::Fresh)
            .unwrap_err(),
        ViewError::WrongProvenance {
            view: "control_plane",
            found: Provenance::LastGoodFile
        }
    );
    assert_eq!(
        base.persisted_file_view(&published, Freshness::Fresh)
            .unwrap_err(),
        ViewError::WrongProvenance {
            view: "persisted_file",
            found: Provenance::ControlPlane
        }
    );
    assert!(
        base.control_plane_view(&published, Freshness::Fresh)
            .is_ok()
    );
    assert!(
        base.persisted_file_view(&restored, Freshness::Fresh)
            .is_ok()
    );
    for snapshot in [&published, &restored] {
        assert!(base.test_view(snapshot, Freshness::Fresh).is_ok());
    }

    // The base is checked first, and the messages name no snapshot text.
    let (other_catalog, other) = world();
    let other_base = self::base(other_catalog, &other.serving, &[]);
    assert_eq!(
        base.control_plane_view(&snapshot(&other_base, 3, []), Freshness::Fresh)
            .unwrap_err(),
        ViewError::ForeignSnapshot
    );
    let message = ViewError::WrongProvenance {
        view: "control_plane",
        found: Provenance::LastGoodFile,
    }
    .to_string();
    assert_eq!(
        message,
        "a control_plane view cannot be built from a snapshot read from the last_good_file"
    );
}

/// Carrying only ever carries a state some snapshot served: from one that
/// served none, a held switch has nothing to keep and serves its safe state.
#[test]
fn carrying_a_snapshot_that_served_no_switch_state_carries_nothing() {
    let (catalog, h) = world();
    let base = base(catalog, &h.serving, &[]);
    let scope = base.validation_scope();
    let empty = snapshot(&base, 10, []);
    let candidate = Candidate::new(&scope)
        .at(version(11))
        .row(undecodable_row("checkout_halt", "switch", "switch", 11))
        .carrying(&empty)
        .unwrap();
    let held = Arc::new(Snapshot::validate(&scope, candidate).unwrap());
    assert_eq!(held.carried().count(), 0);
    let view = base.control_plane_view(&held, Freshness::Fresh).unwrap();
    let decision = view.switch(&h.checkout_halt);
    assert_eq!(
        (decision.value(), decision.layer()),
        (SwitchState::Released, &Layer::Base)
    );
}
