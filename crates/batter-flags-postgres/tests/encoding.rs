//! Writer vectors decode through the existing candidate/snapshot contract.
#![allow(
    clippy::unwrap_used,
    clippy::missing_assert_message,
    reason = "encoding vector assertions"
)]
#[path = "support/fixture.rs"]
mod db_fixture;
#[path = "../../batter-flags-core/tests/support/fixtures.rs"]
mod fixtures;
#[path = "../../batter-flags-core/tests/support/world.rs"]
mod world;
use batter_flags_core::{Freshness, SwitchState, low_level::Snapshot};
use batter_flags_postgres::*;
use db_fixture::*;

#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn stored_writer_vectors_validate_against_core_candidate() {
    let f = Fixture::migrated().await;
    let (catalog, h) = world::world();
    let base = world::base(catalog, &h.serving, &[]);
    let vectors: serde_json::Value = serde_json::from_str(include_str!(
        "../../batter-flags-core/tests/support/state_encoding_1.json"
    ))
    .unwrap();
    for vector in vectors["writer"].as_array().unwrap() {
        let (name, kind, value_type, request) = if vector["shape"] == "switch" {
            let state = if vector["state"] == "engaged" {
                SwitchState::Engaged
            } else {
                SwitchState::Released
            };
            (
                "checkout_halt",
                "switch",
                "switch",
                Write::kill(key("checkout_halt"), state, change(None)),
            )
        } else if vector["variant"] == "express" {
            (
                "checkout_style",
                "mode",
                "enum",
                set("checkout_style", "express", None),
            )
        } else {
            (
                "fast_path",
                "release",
                "toggle",
                set("fast_path", vector["variant"].as_str().unwrap(), None),
            )
        };
        f.seed(name, kind, value_type, "orphan").await;
        assert!(matches!(
            f.store.write(request, deadline()).await.unwrap(),
            WriteOutcome::WrittenUndeclared { .. }
        ));
        let actual: serde_json::Value =
            sqlx::query_scalar("SELECT state FROM flags.flag_state WHERE name=$1")
                .bind(name)
                .fetch_one(&f.pool)
                .await
                .unwrap();
        assert_eq!(
            actual,
            serde_json::from_str::<serde_json::Value>(vector["text"].as_str().unwrap()).unwrap()
        );
        let snapshot = Snapshot::validate(
            &base.validation_scope(),
            f.store
                .load(deadline())
                .await
                .unwrap()
                .candidate(&base.validation_scope()),
        )
        .unwrap();
        assert_eq!(
            snapshot.held(batter_flags_core::Freshness::Fresh).count(),
            0
        );
        let snapshot = std::sync::Arc::new(snapshot);
        let view = base
            .control_plane_view(&snapshot, Freshness::Fresh)
            .unwrap();
        match name {
            "checkout_halt" => assert_eq!(
                view.switch(&h.checkout_halt).variant(),
                vector["state"].as_str().unwrap()
            ),
            "checkout_style" => assert_eq!(view.evaluate(&h.checkout_style).variant(), "express"),
            _ => assert_eq!(
                view.evaluate(&h.fast_path).variant(),
                vector["variant"].as_str().unwrap()
            ),
        }
    }
    f.close().await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn opposite_kind_reader_vectors_refuse_every_mutation_before_other_checks() {
    let vectors: serde_json::Value = serde_json::from_str(include_str!(
        "../../batter-flags-core/tests/support/state_encoding_1.json"
    ))
    .unwrap();
    for name in [
        "a switch state for a flag",
        "a variant for a switch",
        "a switch with a variant field too",
    ] {
        let vector = vectors["reader"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["name"] == name)
            .unwrap();
        assert_eq!(vector["reads"]["held"], "unknown_row_encoding");
        let f = Fixture::migrated().await;
        let (key_name, kind, value_type, request) = match vector["shape"].as_str().unwrap() {
            "flag" => (
                "fast_path",
                "release",
                "toggle",
                set("fast_path", "on", Some(999)),
            ),
            "switch" => (
                "checkout_halt",
                "switch",
                "switch",
                Write::kill(
                    key("checkout_halt"),
                    SwitchState::Engaged,
                    change(Some(999)),
                ),
            ),
            _ => panic!("unexpected shared vector shape"),
        };
        f.seed(key_name, kind, value_type, "live").await;
        let state: serde_json::Value =
            serde_json::from_str(vector["text"].as_str().unwrap()).unwrap();
        let mut tx = f.pool.begin().await.unwrap();
        sqlx::raw_sql("SELECT set_config('batter_flags.actor','fixture',true),set_config('batter_flags.reason','reader vector',true),set_config('batter_flags.guard','unconditional',true); UPDATE flags.flag_control SET version=version+1;")
            .execute(&mut *tx).await.unwrap();
        sqlx::query("INSERT INTO flags.flag_state(name,kind,value_type,state,created_version,updated_version) VALUES ($1,$2,$3,$4,1,1)")
            .bind(key_name).bind(kind).bind(value_type).bind(&state).execute(&mut *tx).await.unwrap();
        tx.commit().await.unwrap();
        assert_held(&f).await;
        // Unknown encoding precedes CAS-above-head, live archive refusal and
        // restore no-op, and remains first even once raw SQL archives the row.
        for phase in [RowPhase::Active, RowPhase::Archived] {
            if let RowPhase::Archived = phase {
                let mut tx = f.pool.begin().await.unwrap();
                sqlx::raw_sql("SELECT set_config('batter_flags.actor','fixture',true),set_config('batter_flags.reason','archive vector',true),set_config('batter_flags.guard','unconditional',true); UPDATE flags.flag_control SET version=version+1;")
                    .execute(&mut *tx).await.unwrap();
                sqlx::query("UPDATE flags.flag_state SET updated_version=2,archived_version=2 WHERE name=$1").bind(key_name).execute(&mut *tx).await.unwrap();
                tx.commit().await.unwrap();
            }
            let before = f.head().await;
            let audit: i64 = sqlx::query_scalar("SELECT count(*) FROM flags.flag_audit")
                .fetch_one(&f.pool)
                .await
                .unwrap();
            let outcomes = [
                f.store.write(request.clone(), deadline()).await.unwrap(),
                f.store
                    .archive(Archive::new(key(key_name), change(Some(999))), deadline())
                    .await
                    .unwrap(),
                f.store
                    .restore(Archive::new(key(key_name), change(Some(999))), deadline())
                    .await
                    .unwrap(),
            ];
            for outcome in outcomes {
                assert!(
                    matches!(outcome, WriteOutcome::Refused(Refusal::UnknownRowEncoding)),
                    "{name}: {outcome:?}"
                );
            }
            assert_eq!(f.head().await, before);
            assert_eq!(
                sqlx::query_scalar::<_, i64>("SELECT count(*) FROM flags.flag_audit")
                    .fetch_one(&f.pool)
                    .await
                    .unwrap(),
                audit
            );
            assert_eq!(
                sqlx::query_scalar::<_, serde_json::Value>(
                    "SELECT state FROM flags.flag_state WHERE name=$1"
                )
                .bind(key_name)
                .fetch_one(&f.pool)
                .await
                .unwrap(),
                state
            );
        }
        f.close().await;
    }
}
enum RowPhase {
    Active,
    Archived,
}
async fn assert_held(f: &Fixture) {
    let (catalog, handles) = world::world();
    let base = world::base(catalog, &handles.serving, &[]);
    let scope = base.validation_scope();
    let snapshot = Snapshot::validate(
        &scope,
        f.store.load(deadline()).await.unwrap().candidate(&scope),
    )
    .unwrap();
    assert_eq!(
        snapshot.held(batter_flags_core::Freshness::Fresh).count(),
        1
    );
}
