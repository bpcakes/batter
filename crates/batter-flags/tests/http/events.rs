//! Gate events are per switch, not per extraction or HTTP completion.
use axum::{http::StatusCode, routing::get};
use batter_axum::GuardedRouter;
use batter_core::telemetry::with_current_dispatch;
use batter_flags::http::AdmittedFlags;
use tracing::Level;
use tracing_subscriber::filter::LevelFilter;

use super::{
    capture::{Capture, Event},
    support::{client, fixture, request, world},
};

fn evaluations(capture: &Capture) -> Vec<Event> {
    capture
        .events()
        .into_iter()
        .filter(|event| event.name == "feature_flag.evaluation")
        .collect()
}

#[tokio::test]
async fn one_gate_evaluation_per_request_at_debug_released_and_info_engaged() {
    for filter in [LevelFilter::DEBUG, LevelFilter::INFO] {
        for state in [
            batter_flags::core::SwitchState::Released,
            batter_flags::core::SwitchState::Engaged,
        ] {
            let (catalog, w) = world();
            let engaged = match state {
                batter_flags::core::SwitchState::Released => vec![],
                batter_flags::core::SwitchState::Engaged => vec![&w.halt],
            };
            let capture = Capture::at(filter);
            let fixture = fixture(catalog, &w, &engaged);
            let app = client(
                fixture.flags().http_gate(&w.halt).unwrap(),
                GuardedRouter::new().route(
                    "/work",
                    get(|_: AdmittedFlags, _: AdmittedFlags| async { "no evaluations" }),
                ),
            )
            .await;
            assert!(evaluations(&capture).is_empty());
            for _ in 0..2 {
                capture.clear();
                let response = capture
                    .within(|| with_current_dispatch(app.request(request())))
                    .await;
                let expected_status = if engaged.is_empty() {
                    StatusCode::OK
                } else {
                    StatusCode::SERVICE_UNAVAILABLE
                };
                assert_eq!(response.status(), expected_status);
                let events = evaluations(&capture);
                if filter == LevelFilter::INFO && engaged.is_empty() {
                    assert!(events.is_empty(), "{events:?}");
                } else {
                    assert_eq!(events.len(), 1, "{events:?}");
                    let event = &events[0];
                    assert_eq!(
                        event.level,
                        if engaged.is_empty() {
                            Level::DEBUG
                        } else {
                            Level::INFO
                        }
                    );
                    assert_eq!(event.target, "batter::flags");
                    assert_eq!(event.field("feature_flag.key"), Some("halt"));
                    assert_eq!(event.field("feature_flag.version"), Some("1"));
                    for field in [
                        "feature_flag.result.value",
                        "error.message",
                        "feature_flag.context.id",
                    ] {
                        assert!(event.field(field).is_none());
                    }
                }
            }
        }
    }
}

#[tokio::test]
async fn capture_only_emits_nothing_and_multi_gate_reads_all_switches_once() {
    let (catalog, w) = world();
    let fixture = fixture(catalog, &w, &[&w.halt]);
    for filter in [LevelFilter::DEBUG, LevelFilter::INFO] {
        let capture = Capture::at(filter);
        for (gate, expected) in [
            (fixture.flags().http_view(), vec![]),
            (
                fixture.flags().http_gate_any(&w.halt, &[&w.other]).unwrap(),
                if filter == LevelFilter::DEBUG {
                    vec![Level::INFO, Level::DEBUG]
                } else {
                    vec![Level::INFO]
                },
            ),
        ] {
            let app = client(
                gate,
                GuardedRouter::new()
                    .route("/work", get(|_: AdmittedFlags| async { "no evaluations" })),
            )
            .await;
            capture.clear();
            let _ = capture
                .within(|| with_current_dispatch(app.request(request())))
                .await;
            let events = evaluations(&capture);
            assert_eq!(
                events.iter().map(|event| event.level).collect::<Vec<_>>(),
                expected,
                "{events:?}"
            );
        }
    }
}
