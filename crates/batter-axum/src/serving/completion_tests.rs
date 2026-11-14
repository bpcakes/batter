use super::*;
use http_body::Body as _;
use std::{io, pin::pin, time::Duration};
use tokio::time::timeout;

/// Whether `future` is still pending after one poll.
async fn pending<F: Future>(future: Pin<&mut F>) -> bool {
    let mut future = future;
    std::future::poll_fn(|cx| Poll::Ready(future.as_mut().poll(cx).is_pending())).await
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
        token: connections.token(),
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

/// A response future that has produced its response no longer holds a token
/// of its own: the response body carries the only one.
#[tokio::test]
async fn a_ready_response_future_hands_its_token_to_the_body() {
    let connections = Connections::new();
    let mut service = ConnectionService {
        inner: tower::service_fn(|_request: Request| async {
            Ok::<_, Infallible>(Response::new(Body::empty()))
        }),
        token: connections.token(),
    };
    let mut responding = pin!(service.call(Request::new(Body::empty())));
    drop(service);
    let response = std::future::poll_fn(|cx| responding.as_mut().poll(cx))
        .await
        .unwrap();
    assert!(responding.token.is_none());
    assert_eq!(connections.0.len(), 1);
    drop(response);
    assert_eq!(connections.0.len(), 0);
}

/// An error return from the server still waits for what its connections hold.
#[tokio::test]
async fn an_error_return_still_waits_for_every_connection() {
    let connections = Connections::new();
    let body = connections.token();
    let mut after =
        pin!(connections.after(async { Err::<(), _>(io::Error::other("serving failed")) }));
    assert!(pending(after.as_mut()).await);
    drop(body);
    let served = timeout(Duration::from_secs(1), after)
        .await
        .expect("the last holder released its token");
    assert_eq!(served.unwrap_err().to_string(), "serving failed");
}
