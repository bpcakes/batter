use super::{Config, router};
use batter::{
    health::{HealthMonitor, HealthPolicy},
    lifecycle::ShutdownHandle,
    settings::{SettingsSource, read_literal},
};
use std::{io::Cursor, time::Duration};

fn source(pairs: &[(&str, &str)]) -> SettingsSource {
    SettingsSource::from_pairs(pairs.iter().map(|(k, v)| ((*k).into(), (*v).into()))).unwrap()
}
fn config(pairs: &[(&str, &str)]) -> Config {
    Config::from_sources(None, SettingsSource::default(), source(pairs)).unwrap()
}
fn app(config: &Config) -> axum::Router {
    let handle = ShutdownHandle::new();
    handle.mark_ready();
    let monitor = HealthMonitor::new(
        HealthPolicy::new(
            Duration::from_secs(1),
            Duration::from_secs(2),
            Duration::from_secs(4),
            Duration::from_secs(1),
        )
        .unwrap(),
        || async { Ok::<_, std::io::Error>(()) },
    );
    router(
        handle,
        config.request_budget,
        monitor.reader(),
        config.bulkhead_capacity,
    )
    .unwrap()
}

#[test]
fn root_source_validation_preserves_defaults_and_explicit_precedence() {
    let default = config(&[]);
    assert_eq!(default.bind.to_string(), "127.0.0.1:3000");
    assert_eq!(default.request_budget, Duration::from_secs(2));
    assert_eq!(default.bulkhead_capacity, 32);
    assert!(default.log_filter.is_none());
    let file = read_literal(
        Cursor::new(
            "BATTER_BIND=127.0.0.1:0\nBATTER_REQUEST_TIMEOUT_MS=bad-shadowed\nRUST_LOG=off",
        ),
        1024,
    )
    .unwrap();
    let loaded = Config::from_sources(
        Some(file),
        source(&[
            ("BATTER_REQUEST_TIMEOUT_MS", "40"),
            ("UNRELATED", "ignored"),
        ]),
        source(&[("BATTER_BULKHEAD_CAPACITY", "2")]),
    )
    .unwrap();
    assert_eq!(loaded.bind.port(), 0);
    assert_eq!(loaded.request_budget, Duration::from_millis(40));
    assert_eq!(loaded.bulkhead_capacity, 2);
    assert_eq!(loaded.log_filter.as_deref(), Some("off"));
    for (key, value) in [
        ("BATTER_REQUEST_TIMEOUT_MS", "0"),
        ("BATTER_REQUEST_TIMEOUT_MS", "-1"),
        ("BATTER_REQUEST_TIMEOUT_MS", "31536000001"),
        ("BATTER_BULKHEAD_CAPACITY", "0"),
        ("BATTER_BULKHEAD_CAPACITY", "18446744073709551615"),
        ("BATTER_BIND", "secret-marker"),
        ("BATTER_UNKNOWN", "secret-marker"),
    ] {
        assert!(
            Config::from_sources(None, SettingsSource::default(), source(&[(key, value)])).is_err()
        );
    }
    assert!(
        Config::from_sources(
            Some(source(&[("UNKNOWN", "secret-marker")])),
            SettingsSource::default(),
            SettingsSource::default()
        )
        .is_err()
    );
    assert!(
        Config::from_sources(
            None,
            source(&[("BATTER_UNKNOWN", "secret-marker")]),
            SettingsSource::default()
        )
        .is_err()
    );
}

#[tokio::test(start_paused = true)]
async fn file_and_environment_deadline_change_actual_work_response() {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;
    for (env, expected) in [
        (SettingsSource::default(), StatusCode::SERVICE_UNAVAILABLE),
        (
            source(&[("BATTER_REQUEST_TIMEOUT_MS", "100")]),
            StatusCode::OK,
        ),
    ] {
        let file = read_literal(Cursor::new("BATTER_REQUEST_TIMEOUT_MS=5"), 1024).unwrap();
        let configured = Config::from_sources(Some(file), env, SettingsSource::default()).unwrap();
        let response = app(&configured)
            .oneshot(Request::builder().uri("/work").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
    }
}

#[tokio::test(start_paused = true)]
async fn actual_router_uses_configured_bulkhead_with_held_work() {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use std::{
        future::{Future, poll_fn},
        task::Poll,
    };
    use tower::ServiceExt;
    for limit in [1, 3] {
        let configured = config(&[("BATTER_BULKHEAD_CAPACITY", &limit.to_string())]);
        let application = app(&configured);
        let request = || Request::builder().uri("/work").body(Body::empty()).unwrap();
        let mut held = Vec::new();
        for _ in 0..limit {
            let mut work = Box::pin(application.clone().oneshot(request()));
            assert!(
                poll_fn(|cx| Poll::Ready(work.as_mut().poll(cx)))
                    .await
                    .is_pending()
            );
            held.push(work);
        }
        let rejected = application.clone().oneshot(request()).await.unwrap();
        assert_eq!(rejected.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body = axum::body::to_bytes(rejected.into_body(), 1024)
            .await
            .unwrap();
        assert!(std::str::from_utf8(&body).unwrap().contains("overloaded"));
        tokio::time::advance(Duration::from_millis(25)).await;
        for work in held {
            assert_eq!(work.await.unwrap().status(), StatusCode::OK);
        }
        assert_eq!(
            application.oneshot(request()).await.unwrap().status(),
            StatusCode::OK
        );
    }
}
