use super::*;

#[tokio::test]
async fn protected_batch_denies_each_policy_without_charging_other_checks() {
    let running = running().await;
    let entered = Arc::new(AtomicUsize::new(0));
    let count = entered.clone();
    let routes = Router::new().route(
        "/work",
        post(move |_: Authenticated<Principal>| {
            count.fetch_add(1, Ordering::SeqCst);
            async { StatusCode::OK }
        }),
    );
    let client = prepare(
        Quota::new(memory()),
        vec![policy("owner", 1), policy("peer", 1)],
        request_policy(&running, 1000),
        routes,
    )
    .in_process();

    let cases = [
        // Charge owner-a and peer one.
        ("Bearer secret-a", "127.0.0.1:10001", StatusCode::OK, 1),
        // Owner-a is exhausted; the failed batch must leave peer two available.
        (
            "Bearer secret-a",
            "127.0.0.2:10002",
            StatusCode::TOO_MANY_REQUESTS,
            1,
        ),
        ("Bearer secret-b", "127.0.0.2:10002", StatusCode::OK, 2),
        // Peer two is exhausted; the failed batch must leave owner-c available.
        (
            "Bearer secret-c",
            "127.0.0.2:10002",
            StatusCode::TOO_MANY_REQUESTS,
            2,
        ),
        ("Bearer secret-c", "127.0.0.3:10003", StatusCode::OK, 3),
    ];
    for (auth, address, status, expected_entries) in cases {
        let response = client
            .request(request("/work", auth), address.parse().unwrap())
            .await;
        assert_eq!(response.status(), status, "{auth} from {address}");
        assert_eq!(entered.load(Ordering::SeqCst), expected_entries);
    }
    finish(running).await;
}

#[test]
fn http_policy_modes_are_validated_at_construction() {
    let build = |policies| {
        let hasher = KeyHasher::new([9; 32]).unwrap();
        HttpQuota::new(
            Quota::new(memory()),
            policies,
            |_input: AuthInput| async { Ok::<_, AuthError>(Principal("owner-a")) },
            move |principal: &Principal, _peer: DirectPeer, policy: &FixedWindowPolicy| {
                hasher
                    .hash_for(policy, principal.0)
                    .into_unbound_subject_key()
            },
        )
    };

    assert!(matches!(
        build(Vec::new()),
        Err(batter_runlimit::http::HttpQuotaConfigError::EmptyPolicies)
    ));
    assert!(matches!(
        build(vec![
            policy("owner", 1),
            policy("peer", 1).with_quota_mode(QuotaMode::Shadow),
        ]),
        Err(
            batter_runlimit::http::HttpQuotaConfigError::MixedQuotaModes {
                first: QuotaMode::Enforce,
                index: 1,
                actual: QuotaMode::Shadow,
            }
        )
    ));
    assert!(build(vec![policy("owner", 1), policy("peer", 1)]).is_ok());
    assert!(
        build(vec![
            policy("owner", 1).with_quota_mode(QuotaMode::Shadow),
            policy("peer", 1).with_quota_mode(QuotaMode::Shadow),
        ])
        .is_ok()
    );
}
