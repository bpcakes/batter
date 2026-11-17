use axum::{Router, routing::get};
use batter_axum::low_level::{register_http, register_http_in, register_http_with_connect_info_in};
use batter_core::{
    cleanup::CleanupBudget,
    lifecycle::{Readiness, ShutdownBudget, Supervisor},
    startup::{Startup, StartupCause, StartupError, StartupOutcome},
};
use std::{
    ops::{Deref, DerefMut},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    task::Poll,
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::oneshot,
    time::timeout,
};

type RegisterHttp = fn(
    &mut Supervisor,
    &'static str,
    TcpListener,
    Router,
) -> Result<(), batter_core::RegistrationError>;

fn cleanup_budget() -> CleanupBudget {
    CleanupBudget::new(
        Duration::from_secs(1),
        Duration::from_secs(1),
        Duration::from_secs(1),
    )
    .unwrap()
}

fn supervisor() -> Supervisor {
    Supervisor::new(
        ShutdownBudget::new(
            Duration::from_millis(30),
            Duration::from_millis(30),
            Duration::from_secs(1),
            cleanup_budget(),
        )
        .unwrap(),
    )
}

struct SupervisorWrapper(Supervisor);
impl Deref for SupervisorWrapper {
    type Target = Supervisor;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl DerefMut for SupervisorWrapper {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

async fn connect(address: std::net::SocketAddr) -> TcpStream {
    let mut client = TcpStream::connect(address).await.unwrap();
    client
        .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    client
}

#[tokio::test]
async fn owned_startup_serves_after_acknowledgement_then_drains_and_runs_cleanup() {
    let cleaned = Arc::new(AtomicBool::new(false));
    let cleanup = cleaned.clone();
    let (address_tx, address_rx) = oneshot::channel();
    let supervisor = supervisor();
    let handle = supervisor.handle();
    assert_eq!(handle.status().readiness(), Readiness::Starting);
    let starting = Startup::scoped(
        supervisor,
        batter_core::operation::OperationOwner::new(Duration::from_secs(2))
            .unwrap()
            .into_context(),
        cleanup_budget(),
        move |scope| {
            Box::pin(async move {
                scope.stage("bind").unwrap();
                let listener = TcpListener::bind("127.0.0.1:0").await?;
                let address = listener.local_addr()?;
                scope
                    .reserve_cleanup("dependency")
                    .unwrap()
                    .register(move || async move {
                        cleanup.store(true, Ordering::SeqCst);
                        Ok(())
                    });
                register_http_in(
                    scope,
                    "http",
                    listener,
                    Router::new().route("/", get(|| async { "served" })),
                )
                .unwrap();
                address_tx.send(address).unwrap();
                Ok::<_, std::io::Error>(())
            })
        },
    )
    .start();
    let running = timeout(Duration::from_secs(3), starting.finish())
        .await
        .unwrap()
        .unwrap();
    timeout(Duration::from_secs(1), handle.status().wait_ready())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(handle.status().readiness(), Readiness::Ready);
    let address = address_rx.await.unwrap();
    let mut client = connect(address).await;
    let mut response = String::new();
    timeout(Duration::from_secs(2), client.read_to_string(&mut response))
        .await
        .unwrap()
        .unwrap();
    handle.request();
    let report = timeout(Duration::from_secs(2), running.wait())
        .await
        .unwrap()
        .unwrap();
    assert!(
        response.starts_with("HTTP/1.1 200 ") && response.ends_with("served"),
        "{response}"
    );
    assert!(report.is_success(), "{report:?}");
    assert_eq!(handle.status().readiness(), Readiness::Stopped);
    assert!(cleaned.load(Ordering::SeqCst));
    assert!(TcpStream::connect(address).await.is_err());
}

#[tokio::test]
async fn invalid_registration_and_unstarted_supervisor_drop_release_the_bound_listener() {
    let _: fn(
        &mut Supervisor,
        &'static str,
        TcpListener,
        Router,
    ) -> Result<(), batter_core::RegistrationError> = register_http;
    let mut wrapped = SupervisorWrapper(supervisor());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    assert!(register_http(&mut wrapped, "", listener, Router::new()).is_err());

    let mut supervisor = supervisor();
    for name in ["", "http"] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let result = register_http(&mut supervisor, name, listener, Router::new());
        if name.is_empty() {
            assert!(result.is_err());
            let rebound = TcpListener::bind(address).await.unwrap();
            drop(rebound);
        } else {
            result.unwrap();
            assert_eq!(supervisor.status().readiness(), Readiness::Starting);
            drop(supervisor);
            let rebound = TcpListener::bind(address).await.unwrap();
            drop(rebound);
            break;
        }
    }
}

/// A layer that counts how often Axum builds the service it wraps.
#[derive(Clone)]
struct CountBuilds(Arc<AtomicUsize>);

impl<S> tower::Layer<S> for CountBuilds {
    type Service = S;

    fn layer(&self, inner: S) -> S {
        self.0.fetch_add(1, Ordering::SeqCst);
        inner
    }
}

#[tokio::test]
async fn plain_server_builds_application_layers_once_across_connections() {
    layers_built_once(register_http).await;
}

#[tokio::test]
async fn peer_server_builds_application_layers_once_across_connections() {
    layers_built_once(register_http_with_connect_info_in).await;
}

async fn layers_built_once(register: RegisterHttp) {
    let builds = Arc::new(AtomicUsize::new(0));
    let mut supervisor = supervisor();
    let handle = supervisor.handle();
    // A layer over a handler route is applied lazily, when Axum prepares the router.
    let app = Router::new()
        .route("/", get(|| async { "served" }))
        .layer(CountBuilds(builds.clone()));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    register(&mut supervisor, "http", listener, app).unwrap();
    let running = supervisor.start();
    timeout(Duration::from_secs(1), handle.status().wait_ready())
        .await
        .unwrap()
        .unwrap();
    let mut observed = Vec::new();
    for _ in 0..3 {
        let mut client = connect(address).await;
        let mut response = String::new();
        timeout(Duration::from_secs(2), client.read_to_string(&mut response))
            .await
            .unwrap()
            .unwrap();
        assert!(response.ends_with("served"), "{response}");
        observed.push(builds.load(Ordering::SeqCst));
    }
    handle.request();
    let report = timeout(Duration::from_secs(2), running.wait())
        .await
        .unwrap()
        .unwrap();
    assert!(report.is_success(), "{report:?}");
    assert_ne!(observed[0], 0, "the counting layer was never built");
    assert_eq!(
        observed, [observed[0]; 3],
        "a later connection rebuilt the router"
    );
}

#[tokio::test]
async fn borrowed_startup_waiter_is_inert_but_owner_drop_releases_listener_before_cleanup() {
    startup_abandonment(register_http).await;
}

#[tokio::test]
async fn peer_server_startup_owner_drop_releases_listener_before_cleanup() {
    startup_abandonment(register_http_with_connect_info_in).await;
}

async fn startup_abandonment(register: RegisterHttp) {
    use std::future::{Future, poll_fn};
    let base = supervisor();
    let handle = base.handle();
    let (bound_tx, bound_rx) = oneshot::channel();
    let (cleanup_tx, mut cleanup_rx) = oneshot::channel();
    let (release_tx, release_rx) = oneshot::channel();
    let starting = Startup::new(
        base,
        batter_core::operation::OperationOwner::new(Duration::from_secs(10))
            .unwrap()
            .into_context(),
        cleanup_budget(),
        move |scope| {
            Box::pin(async move {
                scope.stage("registered").unwrap();
                let listener = TcpListener::bind("127.0.0.1:0").await?;
                let address = listener.local_addr()?;
                scope
                    .supervisor()
                    .on_cleanup("dependency", move || async move {
                        // This fails if startup retains the registered server capture
                        // while closing a resource that the server could still use.
                        let rebound = TcpListener::bind(address).await?;
                        cleanup_tx.send(()).unwrap();
                        release_rx.await.unwrap();
                        drop(rebound);
                        Ok(())
                    })
                    .unwrap();
                register(scope.supervisor(), "http", listener, Router::new()).unwrap();
                bound_tx.send(address).unwrap();
                std::future::pending::<Result<(), std::io::Error>>().await
            })
        },
    )
    .start();
    let observer = starting.observer();
    let address = timeout(Duration::from_secs(2), bound_rx)
        .await
        .unwrap()
        .unwrap();
    let mut waiter = Box::pin(starting.finish());
    assert!(
        poll_fn(|cx| Poll::Ready(waiter.as_mut().poll(cx)))
            .await
            .is_pending()
    );
    tokio::task::yield_now().await;
    assert_eq!(handle.status().readiness(), Readiness::Starting);
    assert!(matches!(
        cleanup_rx.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    ));
    assert!(
        TcpListener::bind(address).await.is_err(),
        "borrowed waiter cannot release the listener"
    );

    drop(waiter);
    timeout(Duration::from_secs(2), &mut cleanup_rx)
        .await
        .unwrap()
        .unwrap();
    let mut completion = Box::pin(observer.wait());
    assert!(
        poll_fn(|cx| Poll::Ready(completion.as_mut().poll(cx)))
            .await
            .is_pending(),
        "report must wait for cleanup"
    );
    release_tx.send(()).unwrap();
    let outcome = timeout(Duration::from_secs(2), completion).await.unwrap();
    assert_startup_drain(outcome);
    assert_eq!(handle.status().readiness(), Readiness::Draining);
    drop(TcpListener::bind(address).await.unwrap());
}

fn assert_startup_drain(outcome: StartupOutcome<std::io::Error>) {
    let StartupOutcome::Failed(StartupError::Failed(report)) = outcome else {
        panic!("expected retained startup drain failure")
    };
    assert!(matches!(report.cause, StartupCause::Draining));
    assert_eq!(report.stage, "registered");
    assert!(report.cleanup.is_success());
    assert_eq!(report.cleanup.records.len(), 1);
    assert_eq!(report.cleanup.records[0].name, "dependency");
}

#[tokio::test]
async fn duplicate_http_registration_releases_only_the_rejected_listener() {
    let mut supervisor = supervisor();
    let first = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let first_address = first.local_addr().unwrap();
    register_http(&mut supervisor, "http", first, Router::new()).unwrap();
    let second = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let second_address = second.local_addr().unwrap();
    assert!(register_http(&mut supervisor, "http", second, Router::new()).is_err());
    drop(TcpListener::bind(second_address).await.unwrap());
    assert!(TcpListener::bind(first_address).await.is_err());
    drop(supervisor);
    drop(TcpListener::bind(first_address).await.unwrap());
}
