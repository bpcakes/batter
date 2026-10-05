use axum::{
    body::Body,
    http::{Request, StatusCode},
    response::IntoResponse,
};
use batter_axum::{
    GuardedRouter, HttpBoundary, HttpObservationLevel, InProcessClient, ProbePath,
    ReadinessDecision, ReadinessPolicy, RequestPolicy, ResponseConstructionBudget,
    default_readiness_level, readiness_status,
};
use batter_core::{
    cleanup::CleanupBudget,
    lifecycle::{ShutdownBudget, Supervisor},
    readiness::{ReadinessCondition, ReadinessUnreadyReason},
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::sync::oneshot;

#[tokio::test]
async fn lifecycle_only_probes_require_approval_driver_acknowledgement_and_conditions() {
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
    let control = supervisor.handle();
    let (start, started) = oneshot::channel();
    let (ack, acknowledged) = oneshot::channel();
    let (release, released) = oneshot::channel();
    supervisor
        .register("application", move |signal| async move {
            started.await.unwrap();
            let signal = signal.acknowledge_started();
            ack.send(()).unwrap();
            released.await.unwrap();
            signal.draining().await;
            Ok(signal.stopped())
        })
        .unwrap();
    let satisfied = Arc::new(AtomicBool::new(false));
    let condition = ReadinessCondition::new("application-state").unwrap();
    let readiness = ReadinessPolicy::lifecycle_only(control.status()).with_condition(condition, {
        let satisfied = satisfied.clone();
        move || satisfied.load(Ordering::Acquire)
    });
    let app = HttpBoundary::new(RequestPolicy::new(
        control.operation_admission(),
        ResponseConstructionBudget::new(second).unwrap(),
    ))
    .with_readiness(ProbePath::new("/empty").unwrap(), readiness.clone())
    .unwrap()
    .with_rendered_readiness(ProbePath::new("/rendered").unwrap(), readiness, |_, _| {
        (StatusCode::CREATED, "application-body").into_response()
    })
    .unwrap()
    .assemble(GuardedRouter::new())
    .await
    .unwrap()
    .in_process();
    let unready = ReadinessDecision::Unready;
    async fn check(app: &InProcessClient, expected: ReadinessDecision) {
        for path in ["/empty", "/rendered"] {
            let response = app
                .clone()
                .request(Request::get(path).body(Body::empty()).unwrap())
                .await;
            assert_eq!(response.status(), readiness_status(expected));
            assert_eq!(
                response.extensions().get::<ReadinessDecision>(),
                Some(&expected)
            );
            assert_eq!(
                response
                    .extensions()
                    .get::<HttpObservationLevel>()
                    .unwrap()
                    .0,
                default_readiness_level(expected)
            );
        }
    }
    check(&app, unready(ReadinessUnreadyReason::Starting)).await;
    let pending = supervisor.start_unapproved();
    check(&app, unready(ReadinessUnreadyReason::Starting)).await;
    let running = pending.approve_readiness();
    check(&app, unready(ReadinessUnreadyReason::Starting)).await;
    start.send(()).unwrap();
    acknowledged.await.unwrap();
    control.status().wait_ready().await.unwrap();
    check(&app, unready(ReadinessUnreadyReason::Condition(condition))).await;
    satisfied.store(true, Ordering::Release);
    check(&app, ReadinessDecision::Ready).await;
    control.request();
    check(&app, unready(ReadinessUnreadyReason::Draining)).await;
    release.send(()).unwrap();
    assert!(running.wait().await.unwrap().is_success());
    check(&app, unready(ReadinessUnreadyReason::Stopped)).await;
}
