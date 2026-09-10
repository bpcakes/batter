use crate::capture::Capture;
use axum::{
    Router,
    body::{Body, to_bytes},
    http::Request,
    middleware,
    routing::get,
};
use batter::{
    cleanup::CleanupBudget,
    health::{HealthMonitor, HealthPolicy, HealthStatus},
    lifecycle::{ShutdownBudget, ShutdownHandle, Supervisor},
};
use batter_axum::{
    HttpObservationLevel, ReadinessPolicy, ReadinessReason, dependency_readiness, operational_http,
};
use std::{
    future::{Future, poll_fn},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    task::Poll,
    time::Duration,
};
use tower::ServiceExt;
use tracing::{Level, instrument::WithSubscriber};

async fn sample(run: &mut (impl Future<Output = ()> + Unpin)) {
    assert!(
        poll_fn(|cx| Poll::Ready(std::pin::Pin::new(&mut *run).poll(cx)))
            .await
            .is_pending()
    );
}

async fn assert_response(policy: &ReadinessPolicy<std::io::Error>, reason: ReadinessReason) {
    assert_eq!(policy.reason(), reason);
    let app: Router = Router::new()
        .route("/ready", get(dependency_readiness::<std::io::Error>))
        .with_state(policy.clone())
        .layer(middleware::from_fn(operational_http));
    let response = app
        .oneshot(
            Request::builder()
                .uri("/ready")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), reason.status());
    assert_eq!(
        response.extensions().get::<ReadinessReason>(),
        Some(&reason)
    );
    assert_eq!(
        response
            .extensions()
            .get::<HttpObservationLevel>()
            .unwrap()
            .0,
        reason.level()
    );
    assert!(to_bytes(response.into_body(), 1).await.unwrap().is_empty());
}

#[tokio::test(start_paused = true)]
async fn decisions_are_read_only_and_distinguish_all_dependency_and_process_states() {
    let capture = Capture::new();
    async {
        let calls = Arc::new(AtomicUsize::new(0));
        let attempts = calls.clone();
        let monitor = HealthMonitor::new(
            HealthPolicy::new(
                Duration::from_secs(1),
                Duration::from_secs(2),
                Duration::from_secs(4),
                Duration::from_secs(1),
            )
            .unwrap(),
            move || {
                let attempt = attempts.fetch_add(1, Ordering::SeqCst);
                async move {
                    match attempt {
                        0 => Err(std::io::Error::other("secret-dependency-cause")),
                        1 => std::future::pending().await,
                        _ => Ok(()),
                    }
                }
            },
        );
        let handle = ShutdownHandle::new();
        let policy = ReadinessPolicy::new(handle.clone(), monitor.reader());
        assert_response(&policy, ReadinessReason::Starting).await;
        handle.mark_ready();
        assert_response(&policy, ReadinessReason::Dependency(HealthStatus::Unknown)).await;
        let probe_handle = ShutdownHandle::new();
        let mut run = Box::pin(monitor.run(probe_handle.signal()));
        sample(&mut run).await;
        assert_response(&policy, ReadinessReason::Dependency(HealthStatus::Failed)).await;
        tokio::time::advance(Duration::from_secs(2)).await;
        sample(&mut run).await;
        tokio::time::advance(Duration::from_secs(1)).await;
        sample(&mut run).await;
        assert_response(&policy, ReadinessReason::Dependency(HealthStatus::TimedOut)).await;
        tokio::time::advance(Duration::from_secs(2)).await;
        sample(&mut run).await;
        assert_response(&policy, ReadinessReason::Ready).await;
        tokio::time::advance(Duration::from_secs(4)).await;
        assert_response(&policy, ReadinessReason::Dependency(HealthStatus::Stale)).await;
        drop(run);
        assert_response(&policy, ReadinessReason::Dependency(HealthStatus::Stopped)).await;
        for _ in 0..100 {
            assert_eq!(
                policy.reason(),
                ReadinessReason::Dependency(HealthStatus::Stopped)
            );
        }
        assert_eq!(calls.load(Ordering::SeqCst), 3);
        handle.request();
        assert_response(&policy, ReadinessReason::Draining).await;
        let cleanup = CleanupBudget::new(
            Duration::from_secs(1),
            Duration::from_secs(1),
            Duration::from_secs(1),
        )
        .unwrap();
        let supervisor = Supervisor::new(
            ShutdownBudget::new(
                Duration::from_secs(1),
                Duration::from_secs(1),
                Duration::from_secs(1),
                cleanup,
            )
            .unwrap(),
        );
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
        let stopped = ReadinessPolicy::new(supervisor.handle(), monitor.reader());
        let report = supervisor.start().wait().await.unwrap();
        assert!(!report.is_success()); // Empty supervisor, used only to observe terminal state.
        assert_response(&stopped, ReadinessReason::Stopped).await;
    }
    .with_subscriber(capture.dispatch.clone())
    .await;
    let text = capture.text();
    let events: Vec<_> = text
        .lines()
        .filter(|line| line.contains("HTTP response boundary finished"))
        .collect();
    assert_eq!(events.len(), 9, "{text}");
    let levels = [
        "INFO", "WARN", "WARN", "WARN", "INFO", "WARN", "WARN", "INFO", "WARN",
    ];
    for (event, level) in events.iter().zip(levels) {
        assert!(event.trim_start().starts_with(level), "{event}");
    }
    assert!(!text.contains("secret-dependency-cause"));
}

#[tokio::test]
async fn explicit_severity_override_preserves_reason_status_body_and_http_outcome() {
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
    let policy = ReadinessPolicy::new(ShutdownHandle::new(), monitor.reader());
    let captures: Vec<_> = (0..2).map(|_| Capture::new()).collect();
    for (policy, capture, level) in [
        (policy.clone(), &captures[0], "INFO"),
        (policy.with_level(|_| Level::ERROR), &captures[1], "ERROR"),
    ] {
        let app: Router = Router::new()
            .route("/ready", get(dependency_readiness::<std::io::Error>))
            .with_state(policy)
            .layer(middleware::from_fn(operational_http));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/ready")
                    .body(Body::empty())
                    .unwrap(),
            )
            .with_subscriber(capture.dispatch.clone())
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), 503);
        assert_eq!(
            response.extensions().get::<ReadinessReason>(),
            Some(&ReadinessReason::Starting)
        );
        assert!(to_bytes(response.into_body(), 1).await.unwrap().is_empty());
        let text = capture.text();
        let events: Vec<_> = text
            .lines()
            .filter(|line| line.contains("HTTP response boundary finished"))
            .collect();
        assert_eq!(events.len(), 1);
        assert!(events[0].trim_start().starts_with(level));
        let fields = events[0]
            .split_once("HTTP response boundary finished")
            .unwrap()
            .1;
        assert!(fields.contains("status=503") && fields.contains("http_outcome=\"server_error\""));
    }
}
