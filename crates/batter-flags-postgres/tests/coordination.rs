//! PostgreSQL 18 coordinated-flip contracts: what refuses one and what says
//! one was accepted.
#![allow(
    clippy::unwrap_used,
    clippy::missing_assert_message,
    reason = "independent database write assertions"
)]
#[path = "support/fixture.rs"]
mod fixture;
use batter_flags_postgres::*;
use fixture::*;

/// A catalog whose only process stopped declares `name` with `discipline`.
async fn declare(f: &Fixture, name: &str, discipline: &str) {
    f.seed(name, "mode", "toggle", "stopped").await;
    sqlx::query("UPDATE flags.flag_catalog_entries SET flip_discipline = $2 WHERE key = $1")
        .bind(name)
        .bind(discipline)
        .execute(&f.pool)
        .await
        .unwrap();
}

fn deferral(outcome: &WriteOutcome) -> &Deferral {
    match outcome {
        WriteOutcome::Written { deferral, .. }
        | WriteOutcome::WrittenUndeclared { deferral, .. } => deferral,
        other => panic!("not written: {other:?}"),
    }
}

/// `write` accepts a coordinated flip and its outcome says so; it used to
/// look exactly like a rolling write. `WritePolicy::STRICT` refuses it.
#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn an_accepted_coordinated_flip_is_recorded_and_strict_refuses_it() {
    let f = Fixture::migrated().await;
    declare(&f, "together", "coordinated").await;
    declare(&f, "rolling", "independent").await;

    let accepted = f
        .store
        .write(set("together", "on", None), deadline())
        .await
        .unwrap();
    assert_eq!(deferral(&accepted).flip, Coordination::Coordinated);
    let rolling = f
        .store
        .write(set("rolling", "on", None), deadline())
        .await
        .unwrap();
    assert_eq!(deferral(&rolling).flip, Coordination::Rolling);

    let (_keep, interrupt) = tokio::sync::oneshot::channel();
    let refused = f
        .store
        .submit(
            Request::Write(set("together", "off", None)),
            WritePolicy::STRICT,
            deadline(),
            interrupt,
        )
        .await
        .unwrap();
    assert!(
        matches!(refused, WriteOutcome::Refused(Refusal::CoordinatedFlip)),
        "{refused:?}"
    );
    f.close().await;
}
