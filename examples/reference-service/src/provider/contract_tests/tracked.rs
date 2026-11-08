use super::*;
use batter::operation::OperationOwner;
use tokio::{io::AsyncReadExt, net::TcpListener};

#[tokio::test]
async fn refused_dispatch_sends_nothing_and_releases_capacity() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client = ProviderClient::new(
        Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap(),
        SecretString::new("fixture-secret"),
    )
    .unwrap();
    let capacity = capacity();
    let parent = OperationOwner::new(Duration::from_secs(10)).unwrap();
    for cancelled in [false, true] {
        let owner = parent.context().child_until(if cancelled {
            parent.context().deadline()
        } else {
            tokio::time::Instant::now()
        });
        if cancelled {
            owner.cancel();
        }
        let request = request();
        let prepared = client
            .prepare_dispatch(&request.idempotency_key(), &request)
            .unwrap();
        let outcome = run_dispatch(&capacity, owner.context(), prepared)
            .await
            .unwrap();
        assert_eq!(
            outcome,
            DispatchOutcome::KnownNotDispatched {
                retry_not_before: Duration::ZERO
            }
        );
        assert!(is_free(&capacity).await);
    }
    assert!(
        tokio::time::timeout(Duration::from_millis(20), listener.accept())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn cancellation_after_request_bytes_remains_uncertain() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client = ProviderClient::new(
        Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap(),
        SecretString::new("fixture-secret"),
    )
    .unwrap();
    let capacity = capacity();
    let owner = OperationOwner::new(Duration::from_secs(10)).unwrap();
    let request = request();
    let prepared = client
        .prepare_dispatch(&request.idempotency_key(), &request)
        .unwrap();
    let (settled, settlement) = tokio::sync::oneshot::channel();
    let exchange = async {
        let result = run_dispatch(&capacity, owner.context(), prepared).await;
        settled.send(()).unwrap();
        result
    };
    let server = async {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut bytes = [0; 4096];
        assert!(stream.read(&mut bytes).await.unwrap() > 0);
        owner.cancel();
        settlement.await.unwrap();
        drop(stream);
    };
    let (result, ()) = tokio::time::timeout(Duration::from_secs(5), async {
        tokio::join!(exchange, server)
    })
    .await
    .unwrap();
    assert!(matches!(
        result,
        Err(ProviderCallError::Interrupted(Interruption::Cancelled))
    ));
    assert!(is_free(&capacity).await);
}
