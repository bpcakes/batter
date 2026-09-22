use super::*;
use axum::response::{IntoResponse, Response};
use batter_axum::quota_observation::{QuotaRecorder, QuotaTerminalFacts};
use runlimit_core::QuotaDenial;
use serde_json::Value;

async fn assert_fixed_response(response: Response, status: StatusCode, code: &str) {
    assert_eq!(response.status(), status);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    let body = to_bytes(response.into_body(), 4096).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body, serde_json::json!({ "code": code }));
}

async fn assert_admission_interruption(response: Response, code: &str) {
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let request_id = response.headers()["x-request-id"]
        .to_str()
        .unwrap()
        .to_owned();
    let body = to_bytes(response.into_body(), 4096).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["code"].as_str(), Some(code));
    assert_eq!(body["request_id"].as_str(), Some(request_id.as_str()));
    assert!(body.get("message").is_some());
}

#[tokio::test]
async fn nested_check_cancellation_uses_admission_renderer_and_original_id() {
    let running = running().await;
    let capture = Capture::new("info");
    let hasher = KeyHasher::new([9; 32]).unwrap();
    let client = HttpQuota::new(
        Quota::new(memory()),
        vec![policy("owner", 1)],
        |input: AuthInput| async move {
            input.context.cancel();
            Ok::<_, AuthError>(Principal("owner-a"))
        },
        move |principal: &Principal, _peer: DirectPeer, policy: &FixedWindowPolicy| {
            hasher
                .hash_for(policy, principal.0)
                .into_unbound_subject_key()
        },
    )
    .unwrap()
    .prepare(request_policy(&running, 1000), routes())
    .in_process();
    let response = capture
        .run(client.request(request("/work", "Bearer secret-a"), peer()))
        .await;
    assert_admission_interruption(response, "operation_cancelled").await;
    assert_eq!(capture.events().len(), 1);
    assert!(capture.events()[0].contains("quota_outcome=\"not_checked\""));
    finish(running).await;
}

#[tokio::test]
async fn cancellation_after_native_grant_keeps_consumption_and_uses_admission_renderer() {
    let running = running().await;
    let capture = Capture::new("info");
    let saved = Arc::new(Mutex::new(None));
    let saved_for_auth = saved.clone();
    let hasher = KeyHasher::new([9; 32]).unwrap();
    let client = HttpQuota::new(
        Quota::new(Backend::new(Mode::CancelSavedThenAllow(saved))),
        vec![policy("owner", 1)],
        move |input: AuthInput| {
            let saved = saved_for_auth.clone();
            async move {
                *saved.lock().unwrap() = Some(input.context);
                Ok::<_, AuthError>(Principal("owner-a"))
            }
        },
        move |principal: &Principal, _peer: DirectPeer, policy: &FixedWindowPolicy| {
            hasher
                .hash_for(policy, principal.0)
                .into_unbound_subject_key()
        },
    )
    .unwrap()
    .prepare(request_policy(&running, 1000), routes())
    .in_process();
    let response = capture
        .run(client.request(request("/work", "Bearer secret-a"), peer()))
        .await;
    assert_admission_interruption(response, "operation_cancelled").await;
    assert_eq!(capture.events().len(), 1);
    assert!(capture.events()[0].contains("quota_outcome=\"allowed\""));
    assert!(capture.events()[0].contains("quota_consumption=\"consumed\""));
    finish(running).await;
}

#[tokio::test(start_paused = true)]
async fn deadline_during_authentication_uses_admission_renderer() {
    let running = running().await;
    let hasher = KeyHasher::new([9; 32]).unwrap();
    let client = HttpQuota::new(
        Quota::new(memory()),
        vec![policy("owner", 1)],
        |_input: AuthInput| async move {
            tokio::time::advance(Duration::from_millis(11)).await;
            Ok::<_, AuthError>(Principal("owner-a"))
        },
        move |principal: &Principal, _peer: DirectPeer, policy: &FixedWindowPolicy| {
            hasher
                .hash_for(policy, principal.0)
                .into_unbound_subject_key()
        },
    )
    .unwrap()
    .prepare(request_policy(&running, 10), routes())
    .in_process();
    let response = client
        .request(request("/work", "Bearer secret-a"), peer())
        .await;
    assert_admission_interruption(response, "deadline_exceeded").await;
    finish(running).await;
}

#[tokio::test]
async fn public_probe_cannot_claim_quota_writer() {
    let running = running().await;
    let capture = Capture::new("info");
    let probe = PublicProbes::new()
        .get("/live", |mut request: Request| async move {
            if let Some(writer) = QuotaRecorder::take(&mut request) {
                writer.start().finish(QuotaTerminalFacts::Allowed);
                "claimed"
            } else {
                "unavailable"
            }
        })
        .unwrap();
    let hasher = KeyHasher::new([9; 32]).unwrap();
    let client = HttpQuota::new(
        Quota::new(memory()),
        vec![policy("owner", 1)],
        |_input: AuthInput| async { Ok::<_, AuthError>(Principal("owner-a")) },
        move |principal: &Principal, _peer: DirectPeer, policy: &FixedWindowPolicy| {
            hasher
                .hash_for(policy, principal.0)
                .into_unbound_subject_key()
        },
    )
    .unwrap()
    .with_public_probes(probe)
    .prepare(request_policy(&running, 1000), routes())
    .in_process();
    let response = capture
        .run(client.request(
            Request::builder().uri("/live").body(Body::empty()).unwrap(),
            peer(),
        ))
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        to_bytes(response.into_body(), 64).await.unwrap(),
        "unavailable"
    );
    assert_eq!(capture.events().len(), 1);
    assert!(capture.events()[0].contains("quota_outcome=\"not_checked\""));
    finish(running).await;
}

#[tokio::test]
async fn empty_public_probe_set_prepares_without_public_handlers() {
    let running = running().await;
    for probes in [PublicProbes::new()] {
        let hasher = KeyHasher::new([9; 32]).unwrap();
        let client = HttpQuota::new(
            Quota::new(memory()),
            vec![policy("owner", 1)],
            |_input: AuthInput| async { Ok::<_, AuthError>(Principal("owner-a")) },
            move |principal: &Principal, _peer: DirectPeer, policy: &FixedWindowPolicy| {
                hasher
                    .hash_for(policy, principal.0)
                    .into_unbound_subject_key()
            },
        )
        .unwrap()
        .with_public_probes(probes)
        .prepare(request_policy(&running, 1000), routes())
        .in_process();
        assert_eq!(
            client
                .request(request("/work", "Bearer secret-a"), peer())
                .await
                .status(),
            StatusCode::OK
        );
        assert_eq!(
            client
                .request(request("/probe/missing", "invalid"), peer())
                .await
                .status(),
            StatusCode::NOT_FOUND
        );
    }
    finish(running).await;
}

#[tokio::test]
async fn nested_protected_fallback_requires_authentication_and_quota_before_body() {
    let running = running().await;
    let capture = Capture::new("info");
    let entered = Arc::new(AtomicUsize::new(0));
    let fallback_entries = entered.clone();
    let routes = Router::new().nest(
        "/api",
        Router::new()
            .route("/work", post(|| async { "work" }))
            .fallback(move |principal: Authenticated<Principal>, body: Bytes| {
                fallback_entries.fetch_add(1, Ordering::SeqCst);
                async move { format!("{}:{}", principal.principal().0, body.len()) }
            }),
    );
    let client = prepare(
        Quota::new(memory()),
        vec![policy("owner", 1)],
        request_policy(&running, 1000),
        routes,
    )
    .in_process();

    let unauthenticated_polls = Arc::new(AtomicUsize::new(0));
    let mut unauthenticated = request("/api/missing", "invalid");
    *unauthenticated.body_mut() = Body::new(CountBody {
        polls: unauthenticated_polls.clone(),
        bytes: Some(Bytes::from_static(b"private")),
    });
    let rejected = capture.run(client.request(unauthenticated, peer())).await;
    assert_fixed_response(
        rejected,
        StatusCode::UNAUTHORIZED,
        "authentication_required",
    )
    .await;
    assert_eq!(unauthenticated_polls.load(Ordering::SeqCst), 0);
    assert_eq!(entered.load(Ordering::SeqCst), 0);

    let mut admitted = request("/api/missing", "Bearer secret-a");
    *admitted.body_mut() = Body::from("private");
    let response = capture.run(client.request(admitted, peer())).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        to_bytes(response.into_body(), 64).await.unwrap(),
        "owner-a:7"
    );
    assert_eq!(entered.load(Ordering::SeqCst), 1);

    let denied_polls = Arc::new(AtomicUsize::new(0));
    let mut denied = request("/api/missing", "Bearer secret-a");
    *denied.body_mut() = Body::new(CountBody {
        polls: denied_polls.clone(),
        bytes: Some(Bytes::from_static(b"private")),
    });
    let response = capture.run(client.request(denied, peer())).await;
    assert_fixed_response(response, StatusCode::TOO_MANY_REQUESTS, "quota_exhausted").await;
    assert_eq!(denied_polls.load(Ordering::SeqCst), 0);
    assert_eq!(entered.load(Ordering::SeqCst), 1);
    assert_eq!(capture.events().len(), 3);
    assert!(capture.events()[0].contains("quota_outcome=\"not_checked\""));
    assert!(capture.events()[1].contains("quota_outcome=\"allowed\""));
    assert!(capture.events()[2].contains("quota_outcome=\"quota_denied\""));
    finish(running).await;
}

#[tokio::test]
async fn protected_method_fallback_uses_the_same_authentication_and_quota_gate() {
    let running = running().await;
    let routes = Router::new()
        .route("/work", post(|| async { "work" }))
        .method_not_allowed_fallback(|principal: Authenticated<Principal>| async move {
            (
                StatusCode::METHOD_NOT_ALLOWED,
                format!("method:{}", principal.principal().0),
            )
        });
    let client = prepare(
        Quota::new(memory()),
        vec![policy("owner", 1)],
        request_policy(&running, 1000),
        routes,
    )
    .in_process();
    let get_request = |auth| {
        Request::builder()
            .method("GET")
            .uri("/work")
            .header(header::AUTHORIZATION, auth)
            .body(Body::empty())
            .unwrap()
    };
    let rejected = client.request(get_request("invalid"), peer()).await;
    assert_fixed_response(
        rejected,
        StatusCode::UNAUTHORIZED,
        "authentication_required",
    )
    .await;
    let admitted = client.request(get_request("Bearer secret-a"), peer()).await;
    assert_eq!(admitted.status(), StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(
        to_bytes(admitted.into_body(), 64).await.unwrap(),
        "method:owner-a"
    );
    let denied = client.request(get_request("Bearer secret-a"), peer()).await;
    assert_fixed_response(denied, StatusCode::TOO_MANY_REQUESTS, "quota_exhausted").await;
    finish(running).await;
}

#[test]
fn public_probe_registration_rejects_route_patterns() {
    for path in ["/{name}", "/{*tail}", "/:legacy"] {
        assert!(matches!(
            PublicProbes::new().get(path, || async { "public" }),
            Err(PublicProbePathError::Pattern)
        ));
    }
    assert!(matches!(
        PublicProbes::new().get("live", || async { "public" }),
        Err(PublicProbePathError::NotAbsolute)
    ));
}

#[tokio::test]
async fn protected_get_colliding_with_public_probe_panics_during_prepare() {
    let running = running().await;
    let protected = Router::new().route("/live", axum::routing::get(|| async { "protected" }));
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        prepare(
            Quota::new(memory()),
            vec![policy("owner", 1)],
            request_policy(&running, 1000),
            protected,
        )
    }));
    assert!(
        result.is_err(),
        "overlapping GET routes must fail at preparation"
    );
    finish(running).await;
}

#[tokio::test]
async fn protected_root_fallback_survives_declared_public_probe() {
    assert!(matches!(
        PublicProbes::new().get("/{*tail}", || async { "public" }),
        Err(PublicProbePathError::Pattern)
    ));
    let running = running().await;
    let hasher = KeyHasher::new([9; 32]).unwrap();
    let business = routes().fallback(|principal: Authenticated<Principal>| async move {
        (
            StatusCode::NOT_FOUND,
            format!("private:{}", principal.principal().0),
        )
    });
    let probes = PublicProbes::new()
        .get("/live", || async { "live" })
        .unwrap();
    let client = HttpQuota::new(
        Quota::new(memory()),
        vec![policy("owner", 1)],
        |input: AuthInput| async move {
            match input.headers.get(header::AUTHORIZATION) {
                Some(value) if value == "Bearer secret-a" => Ok(Principal("owner-a")),
                _ => Err(AuthError),
            }
        },
        move |principal: &Principal, _peer: DirectPeer, policy: &FixedWindowPolicy| {
            hasher
                .hash_for(policy, principal.0)
                .into_unbound_subject_key()
        },
    )
    .unwrap()
    .with_public_probes(probes)
    .prepare(request_policy(&running, 1000), business)
    .in_process();

    let public = client
        .request(
            Request::builder().uri("/live").body(Body::empty()).unwrap(),
            peer(),
        )
        .await;
    assert_eq!(public.status(), StatusCode::OK);
    assert_eq!(to_bytes(public.into_body(), 64).await.unwrap(), "live");
    let rejected = client.request(request("/missing", "invalid"), peer()).await;
    assert_fixed_response(
        rejected,
        StatusCode::UNAUTHORIZED,
        "authentication_required",
    )
    .await;
    let admitted = client
        .request(request("/missing", "Bearer secret-a"), peer())
        .await;
    assert_eq!(admitted.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        to_bytes(admitted.into_body(), 64).await.unwrap(),
        "private:owner-a"
    );
    let denied = client
        .request(request("/missing", "Bearer secret-a"), peer())
        .await;
    assert_fixed_response(denied, StatusCode::TOO_MANY_REQUESTS, "quota_exhausted").await;
    finish(running).await;
}

#[tokio::test]
async fn unavailable_renderer_cannot_publish_quota_before_native_check() {
    let running = running().await;
    let capture = Capture::new("info");
    let backend = Backend::new(Mode::Return(allowed()));
    let claims = Arc::new(AtomicUsize::new(0));
    let claims_in_renderer = claims.clone();
    let http_policy =
        request_policy(&running, 1000).with_failure_renderer(move |failure, parts| {
            assert_eq!(failure, batter_axum::HttpFailure::Unavailable);
            let mut request = Request::from_parts(parts.clone(), Body::empty());
            if let Some(writer) = QuotaRecorder::take(&mut request) {
                claims_in_renderer.fetch_add(1, Ordering::SeqCst);
                writer.start().finish(QuotaTerminalFacts::Allowed);
            }
            failure.status().into_response()
        });
    let client = prepare(
        Quota::new(backend.clone()),
        vec![policy("owner", 1)],
        http_policy,
        routes(),
    )
    .in_process();
    finish(running).await;
    let response = capture
        .run(client.request(request("/work", "Bearer secret-a"), peer()))
        .await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(claims.load(Ordering::SeqCst), 0);
    assert_eq!(backend.calls.load(Ordering::SeqCst), 0);
    assert_eq!(capture.events().len(), 1);
    assert!(capture.events()[0].contains("quota_outcome=\"not_checked\""));
}

#[tokio::test(start_paused = true)]
async fn timeout_renderer_cannot_publish_quota_during_authentication() {
    let running = running().await;
    let capture = Capture::new("info");
    let backend = Backend::new(Mode::Return(allowed()));
    let claims = Arc::new(AtomicUsize::new(0));
    let claims_in_renderer = claims.clone();
    let http_policy = request_policy(&running, 10).with_failure_renderer(move |failure, parts| {
        assert_eq!(failure, batter_axum::HttpFailure::DeadlineExceeded);
        let mut request = Request::from_parts(parts.clone(), Body::empty());
        if let Some(writer) = QuotaRecorder::take(&mut request) {
            claims_in_renderer.fetch_add(1, Ordering::SeqCst);
            writer.start().finish(QuotaTerminalFacts::Allowed);
        }
        failure.status().into_response()
    });
    let hasher = KeyHasher::new([9; 32]).unwrap();
    let client = HttpQuota::new(
        Quota::new(backend.clone()),
        vec![policy("owner", 1)],
        |_input: AuthInput| std::future::pending::<Result<Principal, AuthError>>(),
        move |principal: &Principal, _peer: DirectPeer, policy: &FixedWindowPolicy| {
            hasher
                .hash_for(policy, principal.0)
                .into_unbound_subject_key()
        },
    )
    .unwrap()
    .prepare(http_policy, routes())
    .in_process();
    let response = capture
        .run(client.request(request("/work", "Bearer secret-a"), peer()))
        .await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(claims.load(Ordering::SeqCst), 0);
    assert_eq!(backend.calls.load(Ordering::SeqCst), 0);
    assert_eq!(capture.events().len(), 1);
    assert!(capture.events()[0].contains("quota_outcome=\"not_checked\""));
    finish(running).await;
}

#[tokio::test]
async fn fixed_quota_rejections_have_stable_codes_and_no_store() {
    let running = running().await;
    let owner = policy("owner", 1);
    let client = prepare(
        Quota::new(memory()),
        vec![owner],
        request_policy(&running, 1000),
        routes(),
    )
    .in_process();
    let unauthorized = client.request(request("/work", "invalid"), peer()).await;
    assert!(unauthorized.headers().get(header::RETRY_AFTER).is_none());
    assert_fixed_response(
        unauthorized,
        StatusCode::UNAUTHORIZED,
        "authentication_required",
    )
    .await;
    assert_eq!(
        client
            .request(request("/work", "Bearer secret-a"), peer())
            .await
            .status(),
        StatusCode::OK
    );
    assert_fixed_response(
        client
            .request(request("/work", "Bearer secret-a"), peer())
            .await,
        StatusCode::TOO_MANY_REQUESTS,
        "quota_exhausted",
    )
    .await;

    for (mode, code) in [
        (
            Mode::Return(
                BatchDecision::denied(0, 1, Denial::StorageCapacity { retry_after: None }).unwrap(),
            ),
            "quota_storage_capacity",
        ),
        (
            Mode::Fail(ConsumptionStatus::PossiblyConsumed),
            "quota_backend_failed",
        ),
    ] {
        let client = prepare(
            Quota::new(Backend::new(mode)),
            vec![policy("owner", 1)],
            request_policy(&running, 1000),
            routes(),
        )
        .in_process();
        assert_fixed_response(
            client
                .request(request("/work", "Bearer secret-a"), peer())
                .await,
            StatusCode::SERVICE_UNAVAILABLE,
            code,
        )
        .await;
    }
    finish(running).await;
}

#[tokio::test]
async fn retry_after_preserves_native_ceil_seconds_for_both_denial_kinds() {
    let running = running().await;
    for (delay, expected) in [
        (Duration::ZERO, "0"),
        (Duration::from_nanos(1), "1"),
        (Duration::from_secs(2), "2"),
        (Duration::from_millis(2_001), "3"),
    ] {
        for (denial, status) in [
            (
                Denial::QuotaExceeded(QuotaDenial::new(
                    runlimit_core::Capacity::new(1).unwrap(),
                    delay,
                )),
                StatusCode::TOO_MANY_REQUESTS,
            ),
            (
                Denial::StorageCapacity {
                    retry_after: Some(delay.into()),
                },
                StatusCode::SERVICE_UNAVAILABLE,
            ),
        ] {
            let client = prepare(
                Quota::new(Backend::new(Mode::Return(
                    BatchDecision::denied(0, 1, denial).unwrap(),
                ))),
                vec![policy("owner", 1)],
                request_policy(&running, 1000),
                routes(),
            )
            .in_process();
            let response = client
                .request(request("/work", "Bearer secret-a"), peer())
                .await;
            assert_eq!(response.status(), status);
            assert_eq!(
                response.headers().get(header::RETRY_AFTER).unwrap(),
                expected
            );
        }
    }
    finish(running).await;
}

#[tokio::test]
async fn backend_consumption_projects_every_native_state_into_http_completion() {
    let running = running().await;
    for (native, expected) in [
        (ConsumptionStatus::Consumed, "consumed"),
        (ConsumptionStatus::NotConsumed, "not_consumed"),
        (ConsumptionStatus::PossiblyConsumed, "unknown"),
    ] {
        let capture = Capture::new("info");
        let client = prepare(
            Quota::new(Backend::new(Mode::Fail(native))),
            vec![policy("owner", 1)],
            request_policy(&running, 1000),
            routes(),
        )
        .in_process();
        let response = capture
            .run(client.request(request("/work", "Bearer secret-a"), peer()))
            .await;
        assert_fixed_response(
            response,
            StatusCode::SERVICE_UNAVAILABLE,
            "quota_backend_failed",
        )
        .await;
        assert_eq!(capture.events().len(), 1);
        assert!(capture.events()[0].contains("quota_outcome=\"backend_failed\""));
        assert!(capture.events()[0].contains(&format!("quota_consumption=\"{expected}\"")));
    }
    finish(running).await;
}

#[tokio::test]
async fn unsupported_public_probe_method_has_no_application_fallback() {
    let running = running().await;
    let capture = Capture::new("info");
    let client = prepare(
        Quota::new(memory()),
        vec![policy("owner", 1)],
        request_policy(&running, 1000),
        routes(),
    )
    .in_process();
    let polls = Arc::new(AtomicUsize::new(0));
    let mut unsupported = request("/live", "invalid");
    *unsupported.body_mut() = Body::new(CountBody {
        polls: polls.clone(),
        bytes: Some(Bytes::from_static(b"private")),
    });
    let response = capture.run(client.request(unsupported, peer())).await;
    assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(polls.load(Ordering::SeqCst), 0);
    assert_eq!(capture.events().len(), 1);
    assert!(capture.events()[0].contains("quota_outcome=\"not_checked\""));
    finish(running).await;
}

#[tokio::test]
async fn protected_root_fallback_without_probes_keeps_all_request_gates() {
    let running = running().await;
    let capture = Capture::new("info");
    let hasher = KeyHasher::new([9; 32]).unwrap();
    let entries = Arc::new(AtomicUsize::new(0));
    let fallback_entries = entries.clone();
    let business = routes().fallback(move |principal: Authenticated<Principal>| {
        fallback_entries.fetch_add(1, Ordering::SeqCst);
        async move {
            (
                StatusCode::NOT_FOUND,
                format!("private:{}", principal.principal().0),
            )
        }
    });
    let client = HttpQuota::new(
        Quota::new(memory()),
        vec![policy("owner", 1)],
        |input: AuthInput| async move {
            match input.headers.get(header::AUTHORIZATION) {
                Some(value) if value == "Bearer secret-a" => Ok(Principal("owner-a")),
                _ => Err(AuthError),
            }
        },
        move |principal: &Principal, _peer: DirectPeer, policy: &FixedWindowPolicy| {
            hasher
                .hash_for(policy, principal.0)
                .into_unbound_subject_key()
        },
    )
    .unwrap()
    .prepare(request_policy(&running, 1000), business)
    .in_process();

    let unauthorized = capture
        .run(client.request(request("/missing", "invalid"), peer()))
        .await;
    assert_fixed_response(
        unauthorized,
        StatusCode::UNAUTHORIZED,
        "authentication_required",
    )
    .await;
    assert_eq!(entries.load(Ordering::SeqCst), 0);

    let admitted = capture
        .run(client.request(request("/missing", "Bearer secret-a"), peer()))
        .await;
    assert_eq!(admitted.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        to_bytes(admitted.into_body(), 64).await.unwrap(),
        "private:owner-a"
    );
    assert_eq!(entries.load(Ordering::SeqCst), 1);

    let denied = capture
        .run(client.request(request("/missing", "Bearer secret-a"), peer()))
        .await;
    assert_fixed_response(denied, StatusCode::TOO_MANY_REQUESTS, "quota_exhausted").await;
    assert_eq!(entries.load(Ordering::SeqCst), 1);
    assert_eq!(capture.events().len(), 3);
    assert!(capture.events()[0].contains("quota_outcome=\"not_checked\""));
    assert!(capture.events()[1].contains("quota_outcome=\"allowed\""));
    assert!(capture.events()[2].contains("quota_outcome=\"quota_denied\""));
    finish(running).await;
}
