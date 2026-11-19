//! PostgreSQL 18 contracts for catalogs a strict write cannot judge: each
//! cause is counted apart, because only one of them is fixed by a newer
//! writer.
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
    CatalogBuilder, CivilDate, Expiry, SafeState, SettingsPrefix, StaleRule, Toggle,
    low_level::BaseBuilder,
};
use batter_flags_postgres::*;
use fixture::*;
use std::time::Duration;
use support::report;

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

fn deferral(outcome: &WriteOutcome) -> &Deferral {
    match outcome {
        WriteOutcome::Written { deferral, .. }
        | WriteOutcome::WrittenUndeclared { deferral, .. } => deferral,
        other => panic!("expected a write, got {other:?}"),
    }
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn a_catalog_this_writer_cannot_decode_refuses_a_strict_write() {
    let f = Fixture::migrated().await;
    // A Live declarer whose manifest no version of this writer decodes.
    f.seed("ledger_v2", "mode", "toggle", "live").await;
    sqlx::query("UPDATE flags.flag_catalogs SET manifest = jsonb_build_object('future',(repeat('[',150) || '0' || repeat(']',150))::jsonb)")
        .execute(&f.pool).await.unwrap();
    let refused = strict(&f, "ledger_v2", "on").await;
    assert!(
        matches!(refused, WriteOutcome::Refused(Refusal::InvariantsUnjudged { catalogs }) if (catalogs.undecodable(), catalogs.held_governors()) == (1, 0)),
        "{refused:?}"
    );
    let accepted = submit(
        &f,
        set("ledger_v2", "on", None),
        InvariantPolicy::AcceptHeld,
    )
    .await;
    assert_eq!(
        deferral(&accepted).unjudged.map(|c| c.total().get()),
        Some(1)
    );
    f.close().await;
}

/// A decoded catalog with more governing switches held than the judgement
/// enumerates is unjudged for that cause, not for an undecodable manifest:
/// no newer writer would judge it.
#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn a_catalog_with_too_many_held_governors_is_unjudged_for_that_cause() {
    let f = Fixture::migrated().await;
    let (mut catalog, serving, _) =
        CatalogBuilder::new(SettingsPrefix::new("FLAG_").unwrap(), "serving", "hosted").unwrap();
    let expiry = Expiry::on(CivilDate::new(2027, 3, 31).unwrap());
    let switches = ["halt_a", "halt_b", "halt_c", "halt_d", "halt_e"];
    for (index, name) in switches.iter().enumerate() {
        let switch = catalog
            .kill_switch(name, SafeState::Released, StaleRule::HoldLastKnown)
            .declare()
            .unwrap();
        catalog
            .release_flag(&format!("governed_{index}"), Toggle::Off, expiry)
            .governed_by(&switch)
            .declare()
            .unwrap();
    }
    let base = BaseBuilder::new(catalog.build(), serving).unwrap().finish();
    let labels = ProcessLabels {
        application: ProcessLabel::new("service").unwrap(),
        environment: ProcessLabel::new("hosted").unwrap(),
        build: ProcessLabel::new("governed").unwrap(),
    };
    let cadence = Cadence::new(Duration::from_secs(60), Duration::from_secs(600)).unwrap();
    let mut process = ProcessPresence::new(&f.store, labels, &base, cadence);
    process
        .heartbeat(&report(SnapshotReport::NotLoaded), deadline())
        .await
        .unwrap();
    for name in switches {
        f.unreadable_switch(name).await;
    }
    let refused = strict(&f, "governed_0", "on").await;
    assert!(
        matches!(refused, WriteOutcome::Refused(Refusal::InvariantsUnjudged { catalogs }) if (catalogs.undecodable(), catalogs.held_governors()) == (0, 1)),
        "{refused:?}"
    );
    f.close().await;
}
