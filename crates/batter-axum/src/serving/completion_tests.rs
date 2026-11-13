use super::*;
use http_body::Body as _;
use std::{future::Future, pin::pin, time::Duration};
use tokio::time::timeout;

/// Whether `future` is still pending after one poll.
async fn pending<F: Future>(future: Pin<&mut F>) -> bool {
    let mut future = future;
    poll_fn(|cx| Poll::Ready(future.as_mut().poll(cx).is_pending())).await
}

/// The server waits for the connection's service and for a response body
/// that outlives it, which Hyper destroys last, and forwards the body's size.
#[tokio::test]
async fn destroyed_waits_for_the_service_and_every_response_body() {
    let connections = Connections::new();
    timeout(Duration::from_secs(1), connections.destroyed())
        .await
        .expect("nothing is tracked yet");

    let mut service = ConnectionService {
        inner: tower::service_fn(|_request: Request| async {
            Ok::<_, Infallible>(Response::new(Body::from("body")))
        }),
        token: Token::new(&connections),
    };
    let response = service.call(Request::new(Body::empty())).await.unwrap();
    assert!(pending(pin!(connections.destroyed())).await);
    drop(service);
    assert!(pending(pin!(connections.destroyed())).await);
    assert_eq!(response.body().size_hint().exact(), Some(4));
    assert!(!response.body().is_end_stream());

    let waiting = connections.destroyed();
    drop(response);
    timeout(Duration::from_secs(1), waiting)
        .await
        .expect("the last body released its token");
}

/// Tokens released on another thread while the server waits wake it. This
/// exercises the cross-thread release; the re-check after storing the waker,
/// which covers a release between the first check and that store, is too
/// narrow a window for this loop to hit reliably.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_last_release_on_another_thread_always_wakes_the_waiting_server() {
    for _ in 0..200 {
        let connections = Connections::new();
        let tokens: Vec<_> = (0..4).map(|_| Token::new(&connections)).collect();
        let waiting = tokio::spawn({
            let connections = connections.clone();
            async move { connections.destroyed().await }
        });
        let releasing = std::thread::spawn(move || drop(tokens));
        timeout(Duration::from_secs(1), waiting)
            .await
            .expect("the last release woke the server")
            .unwrap();
        releasing.join().unwrap();
    }
}
