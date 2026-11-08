//! PostgreSQL 18 invariant contracts: a write that leaves a running
//! declarer's `requires`, `conflicts` or `both_or_none` invariant unmet is
//! refused unless the hold is accepted, and a written flag whose governing
//! switch is stored engaged reports the override.
#![allow(
    clippy::unwrap_used,
    clippy::missing_assert_message,
    reason = "independent contract assertions"
)]
#[path = "support/fixture.rs"]
mod fixture;
#[path = "support/presence.rs"]
mod support;
use batter_flags_core::{
    CatalogBuilder, CivilDate, Expiry, InvariantRule, SafeState, SettingsPrefix, StaleRule,
    SwitchState, Toggle, low_level::BaseBuilder,
};
use batter_flags_postgres::*;
use fixture::*;
use std::time::Duration;
use support::report;

/// A `serving` process of a catalog with every relation, heartbeating with
/// `liveness` seconds:
///
/// | flag | relation |
/// | --- | --- |
/// | `ledger_v2` | `on` requires `ledger_audit` `on` |
/// | `stage_three` | `on` requires `ledger_v2` `on` |
/// | `fast_lane` | `on` conflicts with `slow_lane` `on` |
/// | `pair_a` | both or none with `pair_b` |
/// | `serving_side` | serving-only; `on` requires the worker-only `worker_side` `on` |
/// | `new_checkout` | release flag governed by `checkout_halt` (safe state released) |
/// | `checkout_report` | `on` requires `new_checkout` `on` |
/// | `gated` | release flag governed by `gate_halt` (safe state engaged) |
async fn declarer_with(f: &Fixture, liveness: u64) -> ProcessPresence {
    let mut catalog = CatalogBuilder::new(SettingsPrefix::new("FLAG_").unwrap());
    let serving = catalog.process_kind("serving").unwrap();
    let worker = catalog.process_kind("worker").unwrap();
    catalog.environment_class("hosted").unwrap();
    let expiry = Expiry::on(CivilDate::new(2027, 3, 31).unwrap());
    let mut toggle = |key: &str| catalog.mode_flag(key, Toggle::Off).declare().unwrap();
    let ledger_v2 = toggle("ledger_v2");
    let ledger_audit = toggle("ledger_audit");
    let stage_three = toggle("stage_three");
    let fast_lane = toggle("fast_lane");
    let slow_lane = toggle("slow_lane");
    let pair_a = toggle("pair_a");
    let pair_b = toggle("pair_b");
    let checkout_report = toggle("checkout_report");
    let serving_side = catalog
        .mode_flag("serving_side", Toggle::Off)
        .owned_only_by(&serving)
        .declare()
        .unwrap();
    let worker_side = catalog
        .mode_flag("worker_side", Toggle::Off)
        .owned_only_by(&worker)
        .declare()
        .unwrap();
    let halt = catalog
        .kill_switch(
            "checkout_halt",
            SafeState::Released,
            StaleRule::HoldLastKnown,
        )
        .declare()
        .unwrap();
    let gate = catalog
        .kill_switch("gate_halt", SafeState::Engaged, StaleRule::HoldLastKnown)
        .declare()
        .unwrap();
    let new_checkout = catalog
        .release_flag("new_checkout", Toggle::Off, expiry)
        .governed_by(&halt)
        .declare()
        .unwrap();
    catalog
        .release_flag("gated", Toggle::Off, expiry)
        .governed_by(&gate)
        .declare()
        .unwrap();
    catalog
        .requires(&ledger_v2, Toggle::On, &ledger_audit, Toggle::On)
        .unwrap();
    catalog
        .requires(&stage_three, Toggle::On, &ledger_v2, Toggle::On)
        .unwrap();
    catalog
        .requires(&checkout_report, Toggle::On, &new_checkout, Toggle::On)
        .unwrap();
    catalog
        .conflicts(&fast_lane, Toggle::On, &slow_lane, Toggle::On)
        .unwrap();
    catalog.both_or_none(&pair_a, &pair_b).unwrap();
    catalog
        .requires(&serving_side, Toggle::On, &worker_side, Toggle::On)
        .unwrap();
    let base = BaseBuilder::new(catalog.build().unwrap(), serving)
        .unwrap()
        .finish();
    let labels = ProcessLabels {
        application: ProcessLabel::new("service").unwrap(),
        environment: ProcessLabel::new("hosted").unwrap(),
        build: ProcessLabel::new("relations").unwrap(),
    };
    let cadence = Cadence::new(Duration::from_secs(liveness), Duration::from_secs(600)).unwrap();
    let mut process = ProcessPresence::new(labels, &base, cadence);
    f.store
        .heartbeat(&mut process, &report(SnapshotReport::NotLoaded), deadline())
        .await
        .unwrap();
    process
}

/// A Live declarer.
async fn declarer(f: &Fixture) -> ProcessPresence {
    declarer_with(f, 60).await
}

async fn submit(f: &Fixture, write: Write, invariants: InvariantPolicy) -> WriteOutcome {
    let (_keep, interrupt) = tokio::sync::oneshot::channel();
    f.store
        .submit(
            Request::Write(write),
            WritePolicy {
                invariants,
                ..WritePolicy::STRICT
            },
            deadline(),
            interrupt,
        )
        .await
        .unwrap()
}

async fn strict(f: &Fixture, name: &str, variant: &str) -> WriteOutcome {
    submit(f, set(name, variant, None), InvariantPolicy::RefuseUnmet).await
}

fn unmet(
    held: &str,
    rule: InvariantRule,
    (flag, variant): (&str, &str),
    (other, other_variant): (&str, &str),
    basis: Basis,
) -> UnmetInvariant {
    UnmetInvariant {
        held: key(held),
        rule,
        flag: key(flag),
        variant: variant.into(),
        other: key(other),
        other_variant: other_variant.into(),
        basis,
    }
}

/// `ledger_v2` held for lacking `ledger_audit`.
fn ledger(basis: Basis) -> UnmetInvariant {
    unmet(
        "ledger_v2",
        InvariantRule::Requires,
        ("ledger_v2", "on"),
        ("ledger_audit", "on"),
        basis,
    )
}

fn refused_for(outcome: &WriteOutcome) -> &[UnmetInvariant] {
    match outcome {
        WriteOutcome::Refused(Refusal::InvariantUnmet { unmet }) => unmet,
        other => panic!("expected an invariant refusal, got {other:?}"),
    }
}

fn deferral(outcome: &WriteOutcome) -> &Deferral {
    match outcome {
        WriteOutcome::Written { deferral, .. } => deferral,
        other => panic!("expected a write, got {other:?}"),
    }
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn a_dependent_is_refused_until_what_it_requires_is_stored() {
    let f = Fixture::migrated().await;
    let _process = declarer(&f).await;
    let head = f.head().await;
    // Judged against the code default of a flag with no row; nothing written.
    let refused = strict(&f, "ledger_v2", "on").await;
    assert_eq!(refused_for(&refused), [ledger(Basis::CodeDefault)]);
    assert_eq!(f.head().await, head, "a refusal rolls the counter back");
    // The requirement first, then the dependent.
    assert_eq!(
        *deferral(&strict(&f, "ledger_audit", "on").await),
        Deferral::default()
    );
    assert_eq!(
        *deferral(&strict(&f, "ledger_v2", "on").await),
        Deferral::default()
    );
    // Taking the requirement away holds the dependent, which was not held.
    let withdrawn = strict(&f, "ledger_audit", "off").await;
    assert_eq!(refused_for(&withdrawn), [ledger(Basis::StoredRows)]);
    // The unguarded write never refuses, and reports the hold it accepted.
    let accepted = f
        .store
        .write(set("ledger_audit", "off", None), deadline())
        .await
        .unwrap();
    assert_eq!(deferral(&accepted).unmet, [ledger(Basis::StoredRows)]);
    f.close().await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn a_hold_that_cascades_is_refused_where_it_would_newly_hold() {
    let f = Fixture::migrated().await;
    let _process = declarer(&f).await;
    // `ledger_v2` is held already: its requirement was never written.
    let first = submit(
        &f,
        set("ledger_v2", "on", None),
        InvariantPolicy::AcceptHeld,
    )
    .await;
    assert_eq!(deferral(&first).unmet, [ledger(Basis::CodeDefault)]);
    // `stage_three` requires the held `ledger_v2`, so it would be held too;
    // the hold already in place is not reported again.
    let cascaded = strict(&f, "stage_three", "on").await;
    assert_eq!(
        refused_for(&cascaded),
        [unmet(
            "stage_three",
            InvariantRule::Requires,
            ("stage_three", "on"),
            ("ledger_v2", "on"),
            Basis::StoredRows,
        )]
    );
    // Writing the missing requirement releases the hold, so nothing is new.
    assert_eq!(
        *deferral(&strict(&f, "ledger_audit", "on").await),
        Deferral::default()
    );
    assert_eq!(
        *deferral(&strict(&f, "stage_three", "on").await),
        Deferral::default()
    );
    f.close().await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn a_counterpart_row_readers_cannot_decode_is_judged_at_its_code_default() {
    let f = Fixture::migrated().await;
    let _process = declarer(&f).await;
    let mut tx = f.pool.begin().await.unwrap();
    sqlx::raw_sql("SELECT set_config('batter_flags.actor','fixture',true),set_config('batter_flags.reason','newer encoding',true),set_config('batter_flags.guard','unconditional',true); UPDATE flags.flag_control SET version=version+1;")
        .execute(&mut *tx).await.unwrap();
    sqlx::query("INSERT INTO flags.flag_state(name,kind,value_type,state,created_version,updated_version) VALUES ('ledger_audit','mode','toggle','{\"encoding\":2,\"variant\":\"on\"}',1,1)")
        .execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    // Readers hold the newer row at its base, so `ledger_v2` would lack it.
    let refused = strict(&f, "ledger_v2", "on").await;
    assert_eq!(refused_for(&refused), [ledger(Basis::CodeDefault)]);
    f.close().await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn conflicts_and_both_or_none_pairs_need_an_accepted_hold_or_the_right_order() {
    let f = Fixture::migrated().await;
    let _process = declarer(&f).await;
    let conflict = unmet(
        "fast_lane",
        InvariantRule::Conflicts,
        ("fast_lane", "on"),
        ("slow_lane", "on"),
        Basis::StoredRows,
    );
    assert_eq!(
        *deferral(&strict(&f, "slow_lane", "on").await),
        Deferral::default()
    );
    assert_eq!(
        refused_for(&strict(&f, "fast_lane", "on").await),
        std::slice::from_ref(&conflict)
    );
    let held = submit(
        &f,
        set("fast_lane", "on", None),
        InvariantPolicy::AcceptHeld,
    )
    .await;
    assert_eq!(deferral(&held).unmet, [conflict]);

    // A both-or-none pair cannot be turned on one strict write at a time.
    let pair = unmet(
        "pair_a",
        InvariantRule::BothOrNone,
        ("pair_a", "on"),
        ("pair_b", "on"),
        Basis::CodeDefault,
    );
    assert_eq!(
        refused_for(&strict(&f, "pair_a", "on").await),
        std::slice::from_ref(&pair)
    );
    let first = submit(&f, set("pair_a", "on", None), InvariantPolicy::AcceptHeld).await;
    assert_eq!(deferral(&first).unmet, [pair]);
    assert_eq!(
        *deferral(&strict(&f, "pair_b", "on").await),
        Deferral::default()
    );

    // No kind owns both flags of this invariant, so nothing judges it.
    assert_eq!(
        *deferral(&strict(&f, "serving_side", "on").await),
        Deferral::default()
    );
    f.close().await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn only_running_declarers_are_judged_and_a_silent_ones_holds_are_still_reported() {
    let f = Fixture::migrated().await;
    let process = declarer(&f).await;
    f.store.stop(process, deadline()).await.unwrap();
    let written = strict(&f, "ledger_v2", "on").await;
    assert!(
        matches!(&written, WriteOutcome::WrittenUndeclared { deferral, .. } if *deferral == Deferral::default()),
        "{written:?}"
    );
    let _silent = declarer_with(&f, 1).await;
    support::until_silent().await;
    assert_eq!(
        refused_for(&strict(&f, "stage_three", "on").await).len(),
        1,
        "a Silent declarer may still serve"
    );
    let accepted = submit(
        &f,
        set("stage_three", "on", None),
        InvariantPolicy::AcceptHeld,
    )
    .await;
    assert!(
        matches!(&accepted, WriteOutcome::WrittenUndeclared { deferral, .. } if deferral.unmet.len() == 1),
        "{accepted:?}"
    );
    f.close().await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn a_catalog_this_writer_cannot_decode_refuses_a_strict_write() {
    let f = Fixture::migrated().await;
    // A Live declarer whose manifest no version of this writer decodes.
    f.seed("ledger_v2", "mode", "toggle", "live").await;
    let refused = strict(&f, "ledger_v2", "on").await;
    assert!(
        matches!(refused, WriteOutcome::Refused(Refusal::InvariantsUnjudged { catalogs }) if catalogs.get() == 1),
        "{refused:?}"
    );
    let accepted = submit(
        &f,
        set("ledger_v2", "on", None),
        InvariantPolicy::AcceptHeld,
    )
    .await;
    assert_eq!(deferral(&accepted).unjudged_catalogs, 1);
    f.close().await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn a_governed_flag_reports_its_engaged_switch_and_a_kill_switch_is_never_refused() {
    let f = Fixture::migrated().await;
    let _process = declarer(&f).await;
    let kill = |name: &str, state| Write::kill(key(name), state, change(None));
    // Engaged by its safe state alone: no row was ever written.
    assert_eq!(
        deferral(&strict(&f, "gated", "on").await).overridden_by,
        [key("gate_halt")]
    );
    let released = submit(
        &f,
        kill("gate_halt", SwitchState::Released),
        InvariantPolicy::RefuseUnmet,
    )
    .await;
    assert_eq!(*deferral(&released), Deferral::default());
    assert_eq!(
        *deferral(&strict(&f, "gated", "off").await),
        Deferral::default()
    );
    assert_eq!(
        *deferral(&strict(&f, "gated", "on").await),
        Deferral::default()
    );

    // Engaged by its stored row.
    assert_eq!(
        *deferral(&strict(&f, "new_checkout", "on").await),
        Deferral::default()
    );
    assert_eq!(
        *deferral(&strict(&f, "checkout_report", "on").await),
        Deferral::default()
    );
    let engaged = submit(
        &f,
        kill("checkout_halt", SwitchState::Engaged),
        InvariantPolicy::RefuseUnmet,
    )
    .await;
    // Engaging is never refused, and reports what it newly holds.
    assert_eq!(
        deferral(&engaged).unmet,
        [unmet(
            "checkout_report",
            InvariantRule::Requires,
            ("checkout_report", "on"),
            ("new_checkout", "on"),
            Basis::CodeDefault,
        )]
    );
    assert_eq!(
        deferral(&strict(&f, "new_checkout", "off").await).overridden_by,
        []
    );
    assert_eq!(
        deferral(&strict(&f, "new_checkout", "on").await).overridden_by,
        [key("checkout_halt")]
    );
    f.close().await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn a_governor_readers_hold_is_judged_both_engaged_and_released() {
    let f = Fixture::migrated().await;
    let _process = declarer(&f).await;
    assert_eq!(
        *deferral(&strict(&f, "new_checkout", "on").await),
        Deferral::default()
    );
    // A newer build wrote the switch: readers hold it and keep whichever
    // state each last published.
    let mut tx = f.pool.begin().await.unwrap();
    sqlx::raw_sql("SELECT set_config('batter_flags.actor','fixture',true),set_config('batter_flags.reason','newer encoding',true),set_config('batter_flags.guard','unconditional',true); UPDATE flags.flag_control SET version=version+1;")
        .execute(&mut *tx).await.unwrap();
    sqlx::query("INSERT INTO flags.flag_state(name,kind,value_type,state,created_version,updated_version) SELECT 'checkout_halt','switch','switch','{\"encoding\":2,\"switch\":\"engaged\"}',version,version FROM flags.flag_control")
        .execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    // Engaged, `new_checkout` serves its default, so the report is held.
    let refused = strict(&f, "checkout_report", "on").await;
    assert_eq!(
        refused_for(&refused),
        [unmet(
            "checkout_report",
            InvariantRule::Requires,
            ("checkout_report", "on"),
            ("new_checkout", "on"),
            Basis::CodeDefault,
        )]
    );
    let rewritten = strict(&f, "new_checkout", "off").await;
    assert_eq!(*deferral(&rewritten), Deferral::default());
    let on = strict(&f, "new_checkout", "on").await;
    assert_eq!(deferral(&on).overridden_by, [key("checkout_halt")]);
    f.close().await;
}
