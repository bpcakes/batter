//! Public destination policy through the actual resolver and exchange.
use batter_outbound_http::{
    Client, ConnectTimeout, Exchange, Origin, OriginError, ResponseLimit, http::Method,
};
use std::time::Duration;
use tokio::{
    net::TcpListener,
    time::{Instant, timeout},
};

#[test]
fn literal_policy_rejects_special_ranges_and_keeps_existing_origins_compatible() {
    for address in [
        "0.0.0.0",
        "10.1.2.3",
        "127.0.0.1",
        "100.64.0.1",
        "169.254.169.254",
        "172.17.0.1",
        "192.168.1.1",
        "192.0.0.1",
        "192.0.2.1",
        "192.88.99.1",
        "198.18.0.1",
        "198.51.100.1",
        "203.0.113.1",
        "224.0.0.1",
        "255.255.255.255",
        "[::1]",
        "[::]",
        "[::ffff:8.8.8.8]",
        "[64:ff9b::808:808]",
        "[fc00::1]",
        "[fe80::1]",
        "[2001:db8::1]",
        "[2001::1]",
        "[2002:7f00:1::]",
        "[3fff::1]",
    ] {
        let url = format!("https://{address}");
        assert_eq!(
            Origin::public_https(&url),
            Err(OriginError::NonPublicAddress),
            "{address}"
        );
        assert!(Origin::https(&url).is_ok(), "legacy HTTPS policy changed");
    }
    for address in [
        "1.1.1.1",
        "8.8.8.8",
        "[2606:4700:4700::1111]",
        "provider.example",
    ] {
        assert!(Origin::public_https(&format!("https://{address}")).is_ok());
    }
    let origin = Origin::public_https("https://provider.example").unwrap();
    assert!(!format!("{origin:?}").contains("provider.example"));
}

#[tokio::test]
async fn public_dns_policy_survives_cloning_and_stops_before_tcp() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!(
        "https://localhost:{}",
        listener.local_addr().unwrap().port()
    );
    let client = Client::new(
        Origin::public_https(&url).unwrap(),
        ConnectTimeout::new(Duration::from_secs(1)).unwrap(),
    )
    .unwrap();
    let outcome = client
        .clone()
        .request(Method::GET, ["private"])
        .prepare()
        .unwrap()
        .send(
            Instant::now() + Duration::from_secs(2),
            ResponseLimit::new(10).unwrap(),
        )
        .await;
    assert!(matches!(outcome, Exchange::NotDispatched(_)), "{outcome:?}");
    assert!(
        timeout(Duration::from_millis(40), listener.accept())
            .await
            .is_err()
    );

    // Control: ordinary HTTPS still permits this deliberately private origin.
    let client = Client::new(
        Origin::https(&url).unwrap(),
        ConnectTimeout::new(Duration::from_secs(1)).unwrap(),
    )
    .unwrap();
    let exchange = client.request(Method::GET, ["private"]).prepare().unwrap();
    let send = exchange.send(
        Instant::now() + Duration::from_secs(2),
        ResponseLimit::new(10).unwrap(),
    );
    tokio::pin!(send);
    tokio::select! {
        result = listener.accept() => { assert!(result.is_ok()); }
        outcome = &mut send => { panic!("private HTTPS failed before connecting: {outcome:?}"); }
        () = tokio::time::sleep(Duration::from_secs(2)) => { panic!("control did not connect"); }
    }
}
