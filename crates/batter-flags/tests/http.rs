//! The flags adapter's public contracts through protected HTTP assembly.
#![cfg(feature = "axum")]
#![allow(clippy::unwrap_used, reason = "a panic fails the test")]
#![allow(clippy::missing_assert_message, reason = "test names locate failures")]

#[path = "support/capture.rs"]
mod capture;
#[path = "http/events.rs"]
mod events;
#[path = "http/support.rs"]
mod support;
#[path = "../../../test-support/dispatch.rs"]
mod test_dispatch;

use axum::{body::to_bytes, http::StatusCode, response::IntoResponse, routing::get};
use batter_axum::{Gated, GroupGate, GroupPolicy, GuardedRouter, HttpBoundary, HttpFailure};
use batter_flags::{
    GateError,
    core::{Freshness, Source, SwitchState, Toggle, Version, low_level::BaseBuilder},
    http::AdmittedFlags,
    low_level,
    test::TestFlags,
};
use support::{client, fixture, policy, request, world};

#[tokio::test]
async fn capture_only_and_switch_gates_share_one_view_between_extractions() {
    let (catalog, w) = world();
    let fixture = fixture(catalog, &w, &[]);
    for gate in [
        fixture.flags().http_view(),
        fixture.flags().http_gate(&w.halt).unwrap(),
        fixture.flags().http_gate_any(&w.halt, &[]).unwrap(),
    ] {
        let flag = w.flag.clone();
        let halt = w.halt.clone();
        let app = client(
            gate,
            GuardedRouter::new().route(
                "/work",
                get(move |first: AdmittedFlags, second: AdmittedFlags| {
                    let (flag, halt) = (flag.clone(), halt.clone());
                    async move {
                        assert!(std::ptr::eq(first.view(), second.view()));
                        assert_eq!(first.view().evaluate(&flag).value(), Toggle::On);
                        assert_eq!(second.view().switch(&halt).value(), SwitchState::Released);
                        assert_eq!(
                            first.view().source(),
                            Source::Test {
                                version: Version::new(1),
                                freshness: Freshness::Fresh,
                            }
                        );
                        "served"
                    }
                }),
            ),
        )
        .await;
        let response = app.request(request()).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            to_bytes(response.into_body(), 4096).await.unwrap(),
            "served"
        );
    }
}

#[tokio::test]
async fn any_engaged_switch_disables_without_executing_the_handler() {
    use SwitchState::{Engaged, Released};
    for (first, second) in [(Engaged, Released), (Released, Engaged), (Engaged, Engaged)] {
        let (catalog, w) = world();
        let engaged: Vec<_> = [(first, &w.halt), (second, &w.other)]
            .into_iter()
            .filter_map(|(state, switch)| match state {
                Engaged => Some(switch),
                Released => None,
            })
            .collect();
        let fixture = fixture(catalog, &w, &engaged);
        let app = client(
            fixture.flags().http_gate_any(&w.halt, &[&w.other]).unwrap(),
            GuardedRouter::new().route("/work", get(disabled_handler)),
        )
        .await;
        let response = app.request(request()).await;
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(response.headers()["cache-control"], "no-store");
        assert_eq!(
            to_bytes(response.into_body(), 4096).await.unwrap(),
            r#"{"type":"about:blank","title":"Service Unavailable","status":503,"code":"disabled"}"#
        );
    }
}

#[tokio::test]
async fn released_multi_gate_serves_and_capture_only_does_not_gate() {
    let (catalog, w) = world();
    let released = fixture(catalog, &w, &[]);
    let (engaged_catalog, engaged_world) = world();
    let engaged = fixture(engaged_catalog, &engaged_world, &[&engaged_world.halt]);
    for gate in [
        released
            .flags()
            .http_gate_any(&w.halt, &[&w.other])
            .unwrap(),
        engaged.flags().http_view(),
    ] {
        let app = client(
            gate,
            GuardedRouter::new().route("/work", get(|_: AdmittedFlags| async { "served" })),
        )
        .await;
        assert_eq!(app.request(request()).await.status(), StatusCode::OK);
    }
}

#[tokio::test]
async fn stale_rule_and_settings_floor_disable_the_group() {
    let (catalog, w) = world();
    let stale = TestFlags::builder(catalog, w.serving.clone())
        .unwrap()
        .release(&w.other)
        .freshness(Freshness::Stale)
        .build()
        .unwrap();
    let (floor_catalog, floor_world) = world();
    let floor = TestFlags::builder(floor_catalog, floor_world.serving)
        .unwrap()
        .release(&floor_world.other)
        .floor(&floor_world.other)
        .build()
        .unwrap();
    for gate in [
        stale.flags().http_gate(&w.other).unwrap(),
        floor.flags().http_gate(&floor_world.other).unwrap(),
    ] {
        let app = client(
            gate,
            GuardedRouter::new().route("/work", get(disabled_handler)),
        )
        .await;
        assert_eq!(
            app.request(request()).await.status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
    }
}

#[test]
fn constructors_refuse_foreign_unowned_and_duplicate_switches() {
    let (catalog, w) = world();
    let foreign = world().1.halt;
    let fixture = fixture(catalog, &w, &[]);
    let flags = fixture.flags();
    for refusal in [
        flags.http_gate(&foreign),
        flags.http_gate_any(&foreign, &[&w.halt]),
        flags.http_gate_any(&w.halt, &[&foreign]),
    ] {
        assert!(matches!(refusal, Err(GateError::ForeignHandle { key }) if key == *foreign.key()));
    }
    for refusal in [
        flags.http_gate(&w.worker_only),
        flags.http_gate_any(&w.worker_only, &[&w.halt]),
        flags.http_gate_any(&w.halt, &[&w.worker_only]),
    ] {
        assert!(matches!(refusal, Err(GateError::NotOwned { key }) if key == *w.worker_only.key()));
    }
    for refusal in [
        flags.http_gate_any(&w.halt, &[&w.halt]),
        flags.http_gate_any(&w.halt, &[&w.other, &w.other]),
    ] {
        assert!(matches!(refusal, Err(GateError::DuplicateSwitch { .. })));
    }
    assert!(flags.http_gate_any(&w.halt, &[&w.other]).is_ok());
}

async fn assert_internal(app: batter_axum::InProcessClient) {
    let response = app.request(request()).await;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        to_bytes(response.into_body(), 4096).await.unwrap(),
        r#"{"type":"about:blank","title":"Internal Server Error","status":500,"code":"internal_error"}"#
    );
}

fn admitted_routes() -> GuardedRouter {
    GuardedRouter::new().route("/work", get(|_: AdmittedFlags| async { "served" }))
}

#[tokio::test]
async fn absent_or_application_built_native_views_are_sanitized_500() {
    let app = HttpBoundary::new(policy())
        .assemble(admitted_routes())
        .await
        .unwrap()
        .in_process();
    assert_internal(app).await;

    let (catalog, w) = world();
    let native = BaseBuilder::new(catalog, w.serving.clone())
        .unwrap()
        .finish();
    let (catalog, w) = world();
    let flags = fixture(catalog, &w, &[]).flags().clone();
    for gate in [
        GroupGate::new(move || Gated::Serve(native.declared_view())),
        GroupGate::new(move || Gated::Serve(low_level::capture_now(&flags))),
        GroupGate::new(|| Gated::Serve(42_u64)),
    ] {
        assert_internal(client(gate, admitted_routes()).await).await;
    }
}

async fn disabled_handler() -> &'static str {
    panic!("disabled handler ran")
}

#[tokio::test]
async fn disabled_uses_the_group_renderer_but_extractor_failure_stays_sanitized() {
    let (catalog, w) = world();
    let flags = fixture(catalog, &w, &[&w.halt]);
    let custom = policy().with_failure_renderer(|failure, _| {
        assert_eq!(failure, HttpFailure::Disabled);
        (StatusCode::FORBIDDEN, "temporarily unavailable").into_response()
    });
    let app = HttpBoundary::new(GroupPolicy::gated(
        custom,
        flags.flags().http_gate(&w.halt).unwrap(),
    ))
    .assemble(GuardedRouter::new().route("/work", get(disabled_handler)))
    .await
    .unwrap()
    .in_process();
    let response = app.request(request()).await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        to_bytes(response.into_body(), 4096).await.unwrap(),
        "temporarily unavailable"
    );

    let custom = policy().with_failure_renderer(|_, _| {
        panic!("the extractor must use the fixed sanitized response")
    });
    let app = HttpBoundary::new(GroupPolicy::gated(
        custom,
        GroupGate::new(move || Gated::Serve(low_level::capture_now(flags.flags()))),
    ))
    .assemble(admitted_routes())
    .await
    .unwrap()
    .in_process();
    assert_internal(app).await;
}
