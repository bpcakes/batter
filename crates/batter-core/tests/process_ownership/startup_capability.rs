use super::*;

#[tokio::test]
async fn dropping_component_startup_cannot_satisfy_readiness() {
    let mut supervisor = finite_supervisor(1);
    let handle = supervisor.handle();
    let (dropped_tx, dropped_rx) = oneshot::channel();
    supervisor
        .register("unacknowledged", move |startup| async move {
            let shutdown = startup.shutdown().clone();
            // Abandoning is the typed form of dropping an unacknowledged startup.
            let exit = startup.abandon();
            dropped_tx.send(()).unwrap();
            shutdown.draining().await;
            Ok(exit)
        })
        .unwrap();

    let running = supervisor.start();
    dropped_rx.await.unwrap();
    assert_eq!(handle.status().readiness(), Readiness::Starting);
    handle.request();
    assert!(running.shutdown().await.unwrap().is_success());
}
