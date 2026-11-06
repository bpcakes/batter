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
        assert_eq!(snapshot.held().count(), 0);
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
