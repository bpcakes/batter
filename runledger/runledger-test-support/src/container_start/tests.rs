use std::process::{Command, Stdio};

use testcontainers::bollard::models::ContainerInspectResponse;

use super::*;

mod cancellation;

pub(super) struct CleanupGate {
    reached: tokio::sync::mpsc::UnboundedSender<String>,
    release: tokio::sync::Notify,
}

impl CleanupGate {
    pub(super) async fn pause(&self, name: &str) {
        self.reached
            .send(name.to_owned())
            .expect("cleanup observer");
        self.release.notified().await;
    }
}

tokio::task_local! {
    pub(super) static CLEANUP_GATE: std::sync::Arc<CleanupGate>;
}

const CHILD_TEST: &str = "container_start::tests::failed_start_child";
const CHILD_NAME_ENV: &str = "RUNLEDGER_FAILED_START_NAME";
const CHILD_TOKEN_ENV: &str = "RUNLEDGER_FAILED_START_TOKEN";

#[tokio::test]
async fn rejected_start_is_removed_before_normal_test_process_exit() {
    if external_postgres_is_configured() {
        return;
    }
    let attempt = StartAttempt::new();
    let output = run_failed_start_child(&attempt, false).await;
    assert!(
        output.contains("failed-start cleanup completed"),
        "{output}"
    );
    let docker = docker_client_instance().await.expect("Docker client");
    assert_container_absent(&docker, &attempt.name).await;
}

#[tokio::test]
async fn keep_retains_failed_start_and_owned_cleanup_removes_anonymous_volumes() {
    if external_postgres_is_configured() {
        return;
    }
    let attempt = StartAttempt::new();
    let output = run_failed_start_child(&attempt, true).await;
    assert!(
        output.contains("retained by TESTCONTAINERS_COMMAND=keep"),
        "{output}"
    );
    let docker = docker_client_instance().await.expect("Docker client");
    let container = docker
        .inspect_container(&attempt.name, None)
        .await
        .expect("kept failed container");
    assert_eq!(
        container.state.as_ref().and_then(|state| state.running),
        Some(false)
    );
    let volumes = anonymous_volumes(&container);
    assert!(
        !volumes.is_empty(),
        "PostgreSQL image must create an anonymous data volume"
    );
    attempt
        .remove_owned(&docker)
        .await
        .expect("explicitly clean kept test fixture");
    assert_container_absent(&docker, &attempt.name).await;
    for volume in volumes {
        let error = docker
            .inspect_volume(&volume)
            .await
            .expect_err("anonymous volume must be removed");
        assert!(is_not_found(&error), "{error}");
    }
}

#[tokio::test]
async fn name_conflict_does_not_remove_an_unrelated_container() {
    if external_postgres_is_configured() {
        return;
    }
    let original = StartAttempt::new();
    run_failed_start_child(&original, true).await;
    let docker = docker_client_instance().await.expect("Docker client");
    let before = docker
        .inspect_container(&original.name, None)
        .await
        .expect("original container");
    let conflicting = StartAttempt {
        name: original.name.clone(),
        token: StartAttempt::new().token,
        cleanup_gate: None,
    };
    let error = conflicting
        .start(rejected_image(), true)
        .await
        .expect_err("name conflict must fail");
    assert!(error.contains("left untouched"), "{error}");
    let after = docker
        .inspect_container(&original.name, None)
        .await
        .expect("unrelated container survives");
    original
        .remove_owned(&docker)
        .await
        .expect("remove original test fixture");
    assert_eq!(after.id, before.id);
}

#[tokio::test]
#[ignore = "helper entrypoint launched explicitly by failed-start parent tests"]
async fn failed_start_child() {
    let Ok(name) = std::env::var(CHILD_NAME_ENV) else {
        return;
    };
    let attempt = StartAttempt {
        name,
        token: std::env::var(CHILD_TOKEN_ENV).expect("child attempt token"),
        cleanup_gate: None,
    };
    // Docker creates the container and its image-declared anonymous volume,
    // then rejects exec at start. No port collision or host timing is needed.
    let result = attempt
        .start(rejected_image(), should_remove_container())
        .await;
    match result {
        Ok(container) => {
            container
                .rm()
                .await
                .expect("remove unexpectedly started fixture");
            panic!("invalid entrypoint unexpectedly started");
        }
        Err(error) => panic!("expected rejected start: {error}"),
    }
}

fn rejected_image() -> ContainerRequest<GenericImage> {
    let image =
        std::env::var("RUNLEDGER_TEST_PG_IMAGE").unwrap_or_else(|_| "postgres:18".to_owned());
    let (repository, tag) = crate::postgres_container::parse_image_ref(&image);
    GenericImage::new(repository, tag)
        .with_entrypoint("/runledger-deliberately-missing-executable")
        .into()
}

async fn run_failed_start_child(attempt: &StartAttempt, keep: bool) -> String {
    let output = run_child(CHILD_TEST, attempt, keep).await;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        output.status.code(),
        Some(101),
        "child must fail through libtest, not a signal: {text}"
    );
    assert!(text.contains("expected rejected start:"), "{text}");
    assert!(
        text.contains("runledger-deliberately-missing-executable"),
        "{text}"
    );
    text
}

async fn run_child(test: &str, attempt: &StartAttempt, keep: bool) -> std::process::Output {
    let mut child = Command::new(std::env::current_exe().expect("test executable"))
        .args(["--exact", test, "--ignored", "--nocapture"])
        .env(CHILD_NAME_ENV, &attempt.name)
        .env(CHILD_TOKEN_ENV, &attempt.token)
        .env(
            "TESTCONTAINERS_COMMAND",
            if keep { "keep" } else { "remove" },
        )
        .env("RUNLEDGER_TEST_DOCKER_CLI", "runledger-missing-docker-cli")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn failed-start child");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(90);
    while child.try_wait().expect("poll failed-start child").is_none() {
        if tokio::time::Instant::now() >= deadline {
            child.kill().expect("kill timed-out failed-start child");
            child.wait().expect("reap timed-out child");
            panic!("failed-start child exceeded 90 seconds");
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    child
        .wait_with_output()
        .expect("collect failed-start child output")
}

async fn assert_container_absent(docker: &Docker, name: &str) {
    let error = docker
        .inspect_container(name, None)
        .await
        .expect_err("failed container must be absent");
    assert!(is_not_found(&error), "{error}");
}

fn anonymous_volumes(container: &ContainerInspectResponse) -> Vec<String> {
    container
        .mounts
        .iter()
        .flatten()
        .filter_map(|mount| mount.name.clone())
        .collect()
}

fn external_postgres_is_configured() -> bool {
    std::env::var_os("RUNLEDGER_TEST_ADMIN_DATABASE_URL").is_some()
}
