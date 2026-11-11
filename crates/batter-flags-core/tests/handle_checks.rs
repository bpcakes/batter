//! Construction-time handle checks agree with evaluation and emit nothing.
#![allow(clippy::unwrap_used, reason = "a panic fails the test")]
#![allow(clippy::missing_assert_message, reason = "test names locate failures")]

#[path = "support/capture.rs"]
mod capture;
#[path = "support/fixtures.rs"]
mod fixtures;
#[path = "../../../test-support/dispatch.rs"]
mod test_dispatch;
#[path = "support/world.rs"]
mod world;

use batter_flags_core::{
    ErrorCause, Flag, KillSwitch, Reason, SnapshotView,
    low_level::{Base, BaseBuilder, HandleRefusal},
};
use capture::Capture;
use world::{base, world};

fn refusal(cause: ErrorCause) -> HandleRefusal {
    match cause {
        ErrorCause::ForeignCatalog => HandleRefusal::ForeignCatalog,
        ErrorCause::NotOwned => HandleRefusal::NotOwned,
        other => panic!("evaluation reported a cause no check can: {other:?}"),
    }
}

#[test]
fn a_kind_checks_what_it_owns_and_refuses_the_rest() {
    let (catalog, h) = world();
    let base = base(catalog, &h.serving, &[]);
    assert_eq!(base.check_flag(&h.new_checkout), Ok(()));
    assert_eq!(base.check_flag(&h.fast_path), Ok(()));
    assert_eq!(base.check_switch(&h.checkout_halt), Ok(()));
    // `worker_mode` and `export_halt` belong to `worker` alone.
    assert_eq!(
        base.check_flag(&h.worker_mode),
        Err(HandleRefusal::NotOwned)
    );
    assert_eq!(
        base.check_switch(&h.export_halt),
        Err(HandleRefusal::NotOwned)
    );
}

#[test]
fn a_pinner_that_does_not_own_a_flag_is_refused_for_live_use() {
    let (catalog, h) = world();
    // `new_checkout` is `pinned_by` the worker, which replays pins and never
    // evaluates the flag live.
    let base = base(catalog, &h.worker, &[]);
    assert_eq!(
        base.check_flag(&h.new_checkout),
        Err(HandleRefusal::NotOwned)
    );
}

#[test]
fn another_catalogs_handle_is_refused_whatever_its_name() {
    let (catalog, h) = world();
    let (_other_catalog, other) = world();
    let base = base(catalog, &h.serving, &[]);
    // Same keys, same shapes: the catalog identity is what differs.
    assert_eq!(
        base.check_flag(&other.new_checkout),
        Err(HandleRefusal::ForeignCatalog)
    );
    assert_eq!(
        base.check_switch(&other.checkout_halt),
        Err(HandleRefusal::ForeignCatalog)
    );
}

/// A check refuses a flag exactly when evaluating it answers with an error,
/// and for the same cause.
fn agree_on_flag<F: Flag>(base: &Base, view: &SnapshotView, flag: &F) {
    let decision = view.evaluate(flag);
    match (base.check_flag(flag), decision.error()) {
        (Ok(()), None) => assert_ne!(decision.reason(), Reason::Error),
        (Err(refused), Some(cause)) => assert_eq!(refused, refusal(cause)),
        (check, error) => panic!("{}: {check:?} vs {error:?}", flag.key()),
    }
}

fn agree_on_switch(base: &Base, view: &SnapshotView, switch: &KillSwitch) {
    let decision = view.switch(switch);
    match (base.check_switch(switch), decision.error()) {
        (Ok(()), None) => assert_ne!(decision.reason(), Reason::Error),
        (Err(refused), Some(cause)) => assert_eq!(refused, refusal(cause)),
        (check, error) => panic!("{}: {check:?} vs {error:?}", switch.key()),
    }
}

#[test]
fn a_check_refuses_exactly_what_evaluation_answers_with_an_error() {
    for serving in [true, false] {
        let (catalog, h) = world();
        let (_other_catalog, other) = world();
        let kind = if serving { &h.serving } else { &h.worker };
        let base = base(catalog, kind, &[]);
        let view = base.declared_view();
        agree_on_flag(&base, &view, &h.new_checkout);
        agree_on_flag(&base, &view, &h.fast_path);
        agree_on_flag(&base, &view, &h.checkout_style);
        agree_on_flag(&base, &view, &h.debug_panel);
        agree_on_flag(&base, &view, &h.worker_mode);
        agree_on_flag(&base, &view, &other.new_checkout);
        agree_on_flag(&base, &view, &other.worker_mode);
        for switch in [&h.checkout_halt, &h.export_halt, &other.checkout_halt] {
            agree_on_switch(&base, &view, switch);
        }
    }
}

#[test]
fn a_check_emits_no_event_and_counts_no_error() {
    let capture = Capture::new();
    let (catalog, h) = world();
    let (_other_catalog, other) = world();
    let base = base(catalog, &h.serving, &[]);
    capture.within(|| {
        let _ = base.check_flag(&other.new_checkout);
        let _ = base.check_flag(&h.worker_mode);
        let _ = base.check_switch(&h.export_halt);
    });
    assert!(capture.events().is_empty());
    assert_eq!(base.error_count(ErrorCause::ForeignCatalog), 0);
    assert_eq!(base.error_count(ErrorCause::NotOwned), 0);
}

#[test]
fn a_builder_applies_the_rule_before_the_base_is_finished() {
    let (catalog, h) = world();
    let (_other_catalog, other) = world();
    let builder = BaseBuilder::new(catalog, h.serving.clone()).unwrap();
    assert_eq!(builder.check_flag(&h.new_checkout), Ok(()));
    assert_eq!(builder.check_switch(&h.checkout_halt), Ok(()));
    assert_eq!(
        builder.check_flag(&h.worker_mode),
        Err(HandleRefusal::NotOwned)
    );
    assert_eq!(
        builder.check_switch(&h.export_halt),
        Err(HandleRefusal::NotOwned)
    );
    // A same-named switch of another catalog would match `BaseBuilder::switch`
    // by key; the handle check is what tells it apart.
    assert_eq!(
        builder.check_switch(&other.checkout_halt),
        Err(HandleRefusal::ForeignCatalog)
    );
    let finished = builder.finish();
    assert_eq!(finished.check_flag(&h.new_checkout), Ok(()));
}

#[test]
fn refusals_name_no_handle() {
    assert_eq!(
        HandleRefusal::ForeignCatalog.to_string(),
        "the handle belongs to another catalog"
    );
    assert_eq!(
        HandleRefusal::NotOwned.to_string(),
        "this process kind does not own the declaration"
    );
}
