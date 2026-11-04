use super::*;

async fn refused_attempts(builder: reqwest::ClientBuilder) -> usize {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/effects", listener.local_addr().unwrap());
    let (stop, mut stopped) = tokio::sync::oneshot::channel();
    let peer = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut connection = h2::server::handshake(stream).await.unwrap();
        let mut count = 0;
        loop {
            tokio::select! {
                _ = &mut stopped => return count,
                request = connection.accept() => {
                    let (request, mut response) = request.unwrap().unwrap();
                    assert_eq!(request.method(), "POST");
                    count += 1;
                    response.send_reset(h2::Reason::REFUSED_STREAM);
                }
            }
        }
    });
    let client = builder.http2_prior_knowledge().build().unwrap();
    assert!(client.post(endpoint).body("effect").send().await.is_err());
    stop.send(()).unwrap();
    peer.await.unwrap()
}

#[tokio::test]
async fn protocol_refusal_cannot_replay_a_provider_mutation() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let native = reqwest::Client::builder()
            .tls_backend_preconfigured(tls::config())
            .no_proxy();
        assert_eq!(refused_attempts(native).await, 3);
        assert_eq!(refused_attempts(client_builder()).await, 1);
    })
    .await
    .expect("bounded replay regression");
}
