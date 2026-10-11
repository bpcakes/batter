use std::sync::Arc;

use super::*;

const CANCELLATION_CHILD: &str = "container_start::tests::cancellation::cancelled_waiter_child";

#[tokio::test]
async fn cancelled_start_waiter_retains_failed_start_cleanup() {
    if external_postgres_is_configured() {
        return;
    }
    let output = run_child(CANCELLATION_CHILD, &StartAttempt::new(), false).await;
    assert!(
        output.status.success(),
        "cancelled-waiter control failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("cancelled waiter cleanup observed"));
}

#[tokio::test]
#[ignore = "helper entrypoint launched explicitly by cancellation parent test"]
async fn cancelled_waiter_child() {
    if std::env::var_os(CHILD_NAME_ENV).is_none() {
        return;
    }
    let docker = docker_client_instance().await.expect("Docker client");
    let (reached, mut observed) = tokio::sync::mpsc::unbounded_channel();
    let gate = Arc::new(CleanupGate {
        reached,
        release: tokio::sync::Notify::new(),
    });
    // Capture this task-local gate in the real start_postgres attempt, before
    // it moves into its owned task. No other fixture attempt sees this gate.
    let waiter = tokio::spawn(CLEANUP_GATE.scope(gate.clone(), start_postgres(rejected_image())));
    let name = tokio::time::timeout(Duration::from_secs(60), observed.recv())
        .await
        .expect("Docker rejects start before cancellation")
        .expect("attempt reaches failed-start cleanup");
    let container = docker
        .inspect_container(&name, None)
        .await
        .expect("failed container exists before cleanup");
    let id = container.id.as_deref().expect("immutable container ID");
    let volumes = anonymous_volumes(&container);
    assert!(!volumes.is_empty(), "failed start owns an anonymous volume");
    assert_eq!(
        container.state.as_ref().and_then(|state| state.running),
        Some(false)
    );
    assert!(!waiter.is_finished(), "startup waiter is still pending");

    // Cancellation is acknowledged while cleanup is held, so a timing race
    // cannot let cleanup finish before the waiter is destroyed.
    waiter.abort();
    assert!(waiter.await.expect_err("waiter cancelled").is_cancelled());
    assert!(docker.inspect_container(id, None).await.is_ok());
    gate.release.notify_one();

    let removed = tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let container_gone = match docker.inspect_container(id, None).await {
                Err(error) if is_not_found(&error) => true,
                Ok(_) => false,
                Err(error) => panic!("inspect failed container: {error}"),
            };
            let mut volumes_gone = true;
            for volume in &volumes {
                match docker.inspect_volume(volume).await {
                    Err(error) if is_not_found(&error) => {}
                    Ok(_) => volumes_gone = false,
                    Err(error) => panic!("inspect anonymous volume: {error}"),
                }
            }
            if container_gone && volumes_gone {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await;
    // Clean a deliberately regressed candidate before reporting its failure.
    // This happens only after the independent absence observation has failed.
    if removed.is_err() {
        let _ = docker
            .remove_container(
                id,
                Some(RemoveContainerOptions {
                    force: true,
                    v: true,
                    ..Default::default()
                }),
            )
            .await;
    }
    removed.expect("cancelled startup waiter must not abandon container or volume cleanup");
    println!("cancelled waiter cleanup observed");
}
