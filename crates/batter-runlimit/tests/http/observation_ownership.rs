use super::*;

#[derive(Clone, Copy)]
enum PanicStage {
    NativeCheck,
    Handler,
}

async fn panic_retains_observation(stage: PanicStage) {
    let running = running().await;
    let capture = Capture::new("info");
    let backend = Backend::new(match stage {
        PanicStage::NativeCheck => Mode::Panic,
        PanicStage::Handler => Mode::Return(allowed()),
    });
    let handlers = Arc::new(AtomicUsize::new(0));
    let entered = handlers.clone();
    let client = prepare(
        Quota::new(backend.clone()),
        vec![policy("owner", 1)],
        request_policy(&running, 1000),
        Router::new().route(
            "/work",
            post(move |_body: Bytes| async move {
                entered.fetch_add(1, Ordering::SeqCst);
                panic!("secret handler panic");
                #[expect(unreachable_code, reason = "names the handler response type")]
                StatusCode::OK
            }),
        ),
    )
    .in_process();
    let polls = Arc::new(AtomicUsize::new(0));
    let mut request = request("/work", "Bearer secret-a");
    *request.body_mut() = Body::new(CountBody {
        polls: polls.clone(),
        bytes: Some(Bytes::from_static(b"body")),
    });
    let dispatch = capture.dispatch.clone();
    let result = tokio::spawn(async move {
        client
            .request(request, peer())
            .with_subscriber(dispatch)
            .await
    })
    .await;
    assert!(result.unwrap_err().is_panic());
    assert_eq!(backend.calls.load(Ordering::SeqCst), 1);
    assert_eq!(backend.dropped.load(Ordering::SeqCst), 1);
    let events = capture.events();
    assert_eq!(events.len(), 1, "{}", capture.text());
    assert!(events[0].contains("http_outcome=\"dropped\""));
    match stage {
        PanicStage::NativeCheck => {
            assert_eq!(handlers.load(Ordering::SeqCst), 0);
            assert_eq!(polls.load(Ordering::SeqCst), 0);
            assert!(events[0].contains("quota_outcome=\"unresolved\""));
            assert!(events[0].contains("quota_consumption=\"unknown\""));
        }
        PanicStage::Handler => {
            assert_eq!(handlers.load(Ordering::SeqCst), 1);
            assert!(polls.load(Ordering::SeqCst) > 0);
            assert!(events[0].contains("quota_outcome=\"allowed\""));
            assert!(events[0].contains("quota_consumption=\"consumed\""));
        }
    }
    assert!(!capture.text().contains("secret"));
    finish(running).await;
}

#[tokio::test]
async fn native_panic_retains_unknown_consumption_without_body_or_handler() {
    panic_retains_observation(PanicStage::NativeCheck).await;
}

#[tokio::test]
async fn handler_panic_preserves_the_already_published_grant() {
    panic_retains_observation(PanicStage::Handler).await;
}

#[tokio::test]
async fn authentication_failure_leaves_observation_unstarted_and_body_unpolled() {
    let running = running().await;
    let capture = Capture::new("info");
    let backend = Backend::new(Mode::Return(allowed()));
    let handlers = Arc::new(AtomicUsize::new(0));
    let entered = handlers.clone();
    let client = prepare(
        Quota::new(backend.clone()),
        vec![policy("owner", 1)],
        request_policy(&running, 1000),
        Router::new().route(
            "/work",
            post(move |_body: Bytes| async move {
                entered.fetch_add(1, Ordering::SeqCst);
                StatusCode::OK
            }),
        ),
    )
    .in_process();
    let polls = Arc::new(AtomicUsize::new(0));
    let mut request = request("/work", "invalid");
    *request.body_mut() = Body::new(CountBody {
        polls: polls.clone(),
        bytes: Some(Bytes::from_static(b"body")),
    });
    let response = capture.run(client.request(request, peer())).await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(backend.calls.load(Ordering::SeqCst), 0);
    assert_eq!(handlers.load(Ordering::SeqCst), 0);
    assert_eq!(polls.load(Ordering::SeqCst), 0);
    let events = capture.events();
    assert_eq!(events.len(), 1);
    assert!(events[0].contains("quota_outcome=\"not_checked\""));
    assert!(events[0].contains("quota_consumption=\"not_consumed\""));
    finish(running).await;
}
