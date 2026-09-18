use super::*;

#[tokio::test]
async fn quota_drop_destroys_uninvoked_work_factory_under_original_dispatch() {
    struct OnDrop;
    impl Drop for OnDrop {
        fn drop(&mut self) {
            tracing::warn!("uninvoked work factory dropped");
        }
    }
    let original = Capture::new("warn");
    let later = Capture::new("warn");
    let quota = Quota::new(Backend::new(Mode::Pending));
    let policy = policy("owner", 1);
    let checks = [runlimit_core::Check::new(&policy, subject(1))];
    let context = context(1000);
    let guard = OnDrop;
    let mut future = Box::pin(quota.run(
        &context,
        batter_runlimit::Checks::new(&checks).unwrap(),
        move |_| async move {
            drop(guard);
            Ok::<_, Infallible>(())
        },
    ));
    original
        .run(std::future::poll_fn(|cx| {
            assert!(future.as_mut().poll(cx).is_pending());
            Poll::Ready(())
        }))
        .await;
    later
        .run(async move {
            drop(future);
        })
        .await;
    assert!(original.text().contains("uninvoked work factory dropped"));
    assert!(!later.text().contains("uninvoked work factory dropped"));
}

#[tokio::test]
async fn concurrent_requests_have_independent_admission_observations() {
    let running = running().await;
    let capture = Capture::new("info");
    let backend = Backend::new(Mode::Fail(ConsumptionStatus::PossiblyConsumed));
    let failure = prepare(
        Quota::new(backend),
        vec![policy("owner", 1)],
        request_policy(&running, 1000),
        routes(),
    )
    .in_process();
    let success = prepare(
        Quota::new(memory()),
        vec![policy("owner", 1)],
        request_policy(&running, 1000),
        routes(),
    )
    .in_process();
    let (failed, admitted) = capture
        .run(async {
            tokio::join!(
                failure.request(request("/work", "Bearer secret-a"), peer()),
                success.request(request("/work", "Bearer secret-b"), peer())
            )
        })
        .await;
    assert_eq!(failed.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(admitted.status(), StatusCode::OK);
    assert_ne!(
        failed.headers()["x-request-id"],
        admitted.headers()["x-request-id"]
    );
    let events = capture.events();
    assert_eq!(events.len(), 2);
    let failed_event = events
        .iter()
        .find(|event| event.contains("status=503"))
        .unwrap();
    let success_event = events
        .iter()
        .find(|event| event.contains("status=200"))
        .unwrap();
    assert!(failed_event.contains("quota_consumption=\"unknown\""));
    assert!(success_event.contains("quota_consumption=\"consumed\""));
    finish(running).await;
}
