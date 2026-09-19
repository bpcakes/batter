use crate::capture::Capture;
use axum::{
    Router,
    body::{Body, to_bytes},
    http::Request,
    middleware,
    routing::get,
};
use batter_axum::{
    HttpObservationLevel, ReadinessDecision, ReadinessPolicy, default_readiness_level,
    dependency_readiness, operational_http, readiness_status,
};
use batter_core::{
    cleanup::CleanupBudget,
    health::{DependencyUnreadyReason, HealthMonitor, HealthPolicy, HealthStatus},
    lifecycle::{ShutdownBudget, ShutdownHandle, Supervisor},
    readiness::ReadinessUnreadyReason,
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

async fn assert_response(policy: &ReadinessPolicy<std::io::Error>, decision: ReadinessDecision) {
    assert_eq!(policy.decision(), decision);
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
    assert_eq!(response.status(), readiness_status(decision));
    assert_eq!(
        response.extensions().get::<ReadinessDecision>(),
        Some(&decision)
    );
    assert_eq!(response.extensions().get::<ReadinessUnreadyReason>(), None);
    assert_eq!(
        response
            .extensions()
            .get::<HttpObservationLevel>()
            .unwrap()
            .0,
        default_readiness_level(decision)
    );
    assert!(to_bytes(response.into_body(), 1).await.unwrap().is_empty());
}

const fn unready(reason: ReadinessUnreadyReason) -> ReadinessDecision {
    ReadinessDecision::Unready(reason)
}

const fn dependency_unready(reason: DependencyUnreadyReason) -> ReadinessDecision {
    unready(ReadinessUnreadyReason::Dependency(reason))
}

async fn assert_dependency(
    policy: &ReadinessPolicy<std::io::Error>,
    reason: DependencyUnreadyReason,
) {
    assert_response(policy, dependency_unready(reason)).await;
}

const fn starting_is_error(decision: ReadinessDecision) -> Level {
    match decision {
        ReadinessDecision::Unready(ReadinessUnreadyReason::Starting) => Level::ERROR,
        other => default_readiness_level(other),
    }
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
        let (handle, approval) = ShutdownHandle::new_with_readiness_approval();
        let policy = ReadinessPolicy::new(handle.status(), monitor.reader());
        assert_response(&policy, unready(ReadinessUnreadyReason::Starting)).await;
        approval.approve();
        assert_dependency(&policy, DependencyUnreadyReason::Unknown).await;
        let probe_handle = ShutdownHandle::new_unapproved();
        let mut run = Box::pin(monitor.run(probe_handle.signal()));
        sample(&mut run).await;
        assert_dependency(&policy, DependencyUnreadyReason::ProbeFailed).await;
        tokio::time::advance(Duration::from_secs(2)).await;
        sample(&mut run).await;
        tokio::time::advance(Duration::from_secs(1)).await;
        sample(&mut run).await;
        assert_dependency(&policy, DependencyUnreadyReason::ProbeTimedOut).await;
        tokio::time::advance(Duration::from_secs(2)).await;
        sample(&mut run).await;
        assert_response(&policy, ReadinessDecision::Ready).await;
        tokio::time::advance(Duration::from_secs(4)).await;
        assert_dependency(&policy, DependencyUnreadyReason::Stale).await;
        drop(run);
        let writer_stopped = dependency_unready(DependencyUnreadyReason::WriterStopped);
        assert_response(&policy, writer_stopped).await;
        for _ in 0..100 {
            assert_eq!(policy.decision(), writer_stopped);
        }
        assert_eq!(calls.load(Ordering::SeqCst), 3);
        handle.request();
        assert_response(&policy, unready(ReadinessUnreadyReason::Draining)).await;
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
        let stopped = ReadinessPolicy::new(supervisor.status(), monitor.reader());
        let report = supervisor.start().wait().await.unwrap();
        assert!(!report.is_success()); // Empty supervisor, used only to observe terminal state.
        assert_response(&stopped, unready(ReadinessUnreadyReason::Stopped)).await;
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
async fn explicit_severity_override_preserves_decision_status_body_and_http_outcome() {
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
    let policy = ReadinessPolicy::new(ShutdownHandle::new_unapproved().status(), monitor.reader());
    let captures: Vec<_> = (0..2).map(|_| Capture::new()).collect();
    assert_eq!(starting_is_error(ReadinessDecision::Ready), Level::INFO);
    assert_eq!(
        starting_is_error(unready(ReadinessUnreadyReason::Stopped)),
        Level::WARN
    );
    for (policy, capture, level) in [
        (policy.clone(), &captures[0], "INFO"),
        (policy.with_level(starting_is_error), &captures[1], "ERROR"),
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
            response.extensions().get::<ReadinessDecision>(),
            Some(&unready(ReadinessUnreadyReason::Starting))
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

#[tokio::test]
async fn supervised_monitor_stop_during_drain_stays_info_until_process_stops() {
    let capture = Capture::new();
    async {
        let second = Duration::from_secs(1);
        let mut supervisor = Supervisor::new(
            ShutdownBudget::new(
                second,
                second,
                second,
                CleanupBudget::new(second, second, second).unwrap(),
            )
            .unwrap(),
        );
        let handle = supervisor.handle();
        let calls = Arc::new(AtomicUsize::new(0));
        let called = calls.clone();
        let monitor = HealthMonitor::new(
            HealthPolicy::new(
                second,
                Duration::from_secs(30),
                Duration::from_secs(60),
                second,
            )
            .unwrap(),
            move || {
                called.fetch_add(1, Ordering::SeqCst);
                async { Ok::<_, std::io::Error>(()) }
            },
        );
        let reader = monitor.reader();
        let policy = ReadinessPolicy::new(handle.status(), reader.clone());
        let (stopped_tx, stopped_rx) = tokio::sync::oneshot::channel();
        supervisor
            .register("health", move |startup| async move {
                let signal = startup.acknowledge_started();
                monitor.run(signal).await;
                stopped_tx.send(()).unwrap();
                Ok(())
            })
            .unwrap();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        supervisor
            .register("drain-control", move |startup| async move {
                let signal = startup.acknowledge_started();
                signal.draining().await;
                release_rx.await.unwrap();
                Ok(())
            })
            .unwrap();
        let running = supervisor.start();
        tokio::time::timeout(second, handle.status().wait_ready())
            .await
            .unwrap()
            .unwrap();
        // The sole writer has sampled before waiting for its next interval.
        assert_eq!(reader.snapshot().status(), HealthStatus::Healthy);
        assert_response(&policy, ReadinessDecision::Ready).await;
        handle.request();
        tokio::time::timeout(second, stopped_rx)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(reader.snapshot().status(), HealthStatus::Stopped);
        let draining = unready(ReadinessUnreadyReason::Draining);
        assert_response(&policy, draining).await;
        for _ in 0..100 {
            assert_eq!(policy.decision(), draining);
        }
        release_tx.send(()).unwrap();
        let report = tokio::time::timeout(second, running.wait())
            .await
            .unwrap()
            .unwrap();
        assert!(report.is_success(), "{report:?}");
        assert_response(&policy, unready(ReadinessUnreadyReason::Stopped)).await;
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
    .with_subscriber(capture.dispatch.clone())
    .await;
    let text = capture.text();
    let events: Vec<_> = text
        .lines()
        .filter(|line| line.contains("HTTP response boundary finished"))
        .collect();
    assert_eq!(events.len(), 3, "{text}");
    for (line, level) in events.iter().zip(["INFO", "INFO", "WARN"]) {
        assert!(line.trim_start().starts_with(level), "{line}");
    }
}
