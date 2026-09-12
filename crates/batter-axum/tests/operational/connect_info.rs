use axum::{
    Router,
    extract::{ConnectInfo, Request},
    middleware::{Next, from_fn},
    response::Response,
    routing::get,
};
use batter::{
    RegistrationError,
    cleanup::CleanupBudget,
    lifecycle::{Readiness, ShutdownBudget, Supervisor},
    operation::OperationContext,
    startup::Startup,
};
use batter_axum::register_http_with_connect_info_in;
use std::{net::SocketAddr, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::oneshot,
    time::timeout,
};

const WAIT: Duration = Duration::from_secs(3);

fn cleanup_budget() -> CleanupBudget {
    CleanupBudget::new(WAIT, WAIT, WAIT).unwrap()
}

fn supervisor() -> Supervisor {
    Supervisor::new(ShutdownBudget::new(WAIT, WAIT, WAIT, cleanup_budget()).unwrap())
}

async fn admission_peer(
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    request: Request,
    next: Next,
) -> Response {
    let mut response = next.run(request).await;
    response
        .headers_mut()
        .insert("x-admission-peer", peer.to_string().parse().unwrap());
    response
}

async fn assert_peer(mut client: TcpStream) {
    // Independent oracle: neither expected address nor port comes from the server.
    let expected = client.local_addr().unwrap();
    client.write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\nForwarded: for=192.0.2.40:4567\r\nX-Forwarded-For: 192.0.2.41\r\nX-Real-IP: 192.0.2.42\r\n\r\n").await.unwrap();
    let mut response = String::new();
    timeout(WAIT, client.read_to_string(&mut response))
        .await
        .unwrap()
        .unwrap();
    let (headers, body) = response.split_once("\r\n\r\n").unwrap();
    assert!(headers.starts_with("HTTP/1.1 200 "), "{response}");
    assert!(
        headers
            .lines()
            .any(|line| line == format!("x-admission-peer: {expected}")),
        "{response}"
    );
    assert_eq!(body, expected.to_string());
}

#[tokio::test]
async fn protected_startup_supplies_each_socket_peer_to_admission_and_handler() {
    let supervisor = supervisor();
    let handle = supervisor.handle();
    let (address_tx, address_rx) = oneshot::channel();
    let (cleanup_tx, cleanup_rx) = oneshot::channel();
    let mut starting = Startup::scoped(
        supervisor,
        OperationContext::new(WAIT).unwrap(),
        cleanup_budget(),
        move |scope| {
            Box::pin(async move {
                let listener = TcpListener::bind("127.0.0.1:0").await?;
                let address = listener.local_addr()?;
                scope
                    .reserve_cleanup("dependency")
                    .unwrap()
                    .register(move || async move {
                        // Successful cleanup follows release of the serving listener.
                        let _rebound = TcpListener::bind(address).await?;
                        cleanup_tx.send(()).unwrap();
                        Ok(())
                    });
                let router = Router::new()
                    .route(
                        "/",
                        get(|ConnectInfo(peer): ConnectInfo<SocketAddr>| async move {
                            peer.to_string()
                        }),
                    )
                    .layer(from_fn(admission_peer));
                register_http_with_connect_info_in(scope, "http", listener, router).unwrap();
                address_tx.send(address).unwrap();
                Ok::<_, std::io::Error>(())
            })
        },
    )
    .start();
    let running = timeout(WAIT, starting.wait()).await.unwrap().unwrap();
    timeout(WAIT, handle.wait_ready()).await.unwrap().unwrap();
    assert_eq!(handle.readiness(), Readiness::Ready);
    let address = address_rx.await.unwrap();
    let first = timeout(WAIT, TcpStream::connect(address))
        .await
        .unwrap()
        .unwrap();
    let second = timeout(WAIT, TcpStream::connect(address))
        .await
        .unwrap()
        .unwrap();
    assert_ne!(first.local_addr().unwrap(), second.local_addr().unwrap());
    tokio::join!(assert_peer(first), assert_peer(second));
    handle.request();
    let report = timeout(WAIT, running.wait()).await.unwrap().unwrap();
    assert!(report.is_success(), "{report:?}");
    assert_eq!(report.cleanup.records.len(), 1);
    assert_eq!(report.cleanup.records[0].name, "dependency");
    cleanup_rx.await.unwrap();
    assert_eq!(handle.readiness(), Readiness::Stopped);
    assert!(TcpStream::connect(address).await.is_err());
}

#[tokio::test]
async fn peer_registration_rejects_names_and_releases_only_rejected_or_abandoned_listeners() {
    let mut supervisor = supervisor();
    let invalid = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let invalid_address = invalid.local_addr().unwrap();
    assert!(matches!(
        register_http_with_connect_info_in(&mut supervisor, "", invalid, Router::new()),
        Err(RegistrationError::InvalidName)
    ));
    drop(TcpListener::bind(invalid_address).await.unwrap());
    let accepted = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let accepted_address = accepted.local_addr().unwrap();
    register_http_with_connect_info_in(&mut supervisor, "http", accepted, Router::new()).unwrap();
    let rejected = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let rejected_address = rejected.local_addr().unwrap();
    assert!(matches!(
        register_http_with_connect_info_in(&mut supervisor, "http", rejected, Router::new()),
        Err(RegistrationError::Duplicate("http"))
    ));
    drop(TcpListener::bind(rejected_address).await.unwrap());
    assert!(TcpListener::bind(accepted_address).await.is_err());
    assert_eq!(supervisor.handle().readiness(), Readiness::Starting);
    drop(supervisor);
    drop(TcpListener::bind(accepted_address).await.unwrap());
}
